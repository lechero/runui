//! MCP requests against the binary, in a home with no personal runs or config.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Client {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Client {
    fn request(&mut self, id: i32, method: &str, params: Value) -> Value {
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.input, "{request}").unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        assert!(self.output.read_line(&mut line).unwrap() > 0, "MCP server exited early");
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["jsonrpc"], "2.0");
        assert_eq!(reply["id"], id);
        reply
    }

    fn notify(&mut self, method: &str) {
        writeln!(self.input, "{}", json!({"jsonrpc": "2.0", "method": method})).unwrap();
        self.input.flush().unwrap();
    }

    fn tool(&mut self, id: i32, name: &str, arguments: Value) -> Value {
        self.request(id, "tools/call", json!({"name": name, "arguments": arguments}))["result"].clone()
    }
}

fn id_of(run: &Value) -> String {
    run["structuredContent"]["id"].as_str().unwrap().to_string()
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_discovers_starts_reads_and_stops_a_run() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("project");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("Makefile"), "build: ## Build the app\n\techo built\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_runui"))
        .args(["mcp", "-C", dir.to_str().unwrap()])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let output = BufReader::new(child.stdout.take().unwrap());
    let mut client = Client { child, input, output };

    let before = client.request(1, "tools/list", json!({}));
    assert_eq!(before["error"]["code"], -32000);
    let probe = client.request(11, "server/discover", json!({}));
    assert_eq!(probe["error"]["code"], -32601);
    let init = client.request(
        2,
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    client.notify("notifications/initialized");
    let tools = client.request(3, "tools/list", json!({}));
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 6);

    let commands = client.tool(4, "list_commands", json!({}));
    assert_eq!(commands["isError"], false);
    assert!(commands["structuredContent"]["commands"].as_array().unwrap().iter().any(|c| c["command"] == "make build"));

    assert_eq!(commands["structuredContent"]["complete"], true);
    assert_eq!(commands["structuredContent"]["diagnostics"], json!([]));
    fs::write(
        dir.join("Taskfile.yml"),
        "version: '3'\nincludes:\n  remote: https://example.invalid/Taskfile.yml\ntasks:\n  check: echo check\n",
    )
    .unwrap();
    let partial = client.tool(40, "list_commands", json!({}));
    assert_eq!(partial["isError"], false);
    let data = &partial["structuredContent"];
    assert_eq!(data["complete"], false);
    assert_eq!(data["errors"], json!([]));
    assert_eq!(data["diagnostics"][0]["source"], "task");
    assert!(data["diagnostics"][0]["message"].as_str().unwrap().contains("not inspected"));
    assert!(data["commands"].as_array().unwrap().iter().any(|command| command["command"] == "make build"));
    fs::remove_file(dir.join("Taskfile.yml")).unwrap();

    // The CLI and MCP resolve the same stable catalog ID and fingerprint.
    let page = client.tool(41, "list_commands", json!({"query":"source:make build","limit":1}));
    let definition = &page["structuredContent"]["commands"][0];
    let definition_id = definition["id"].as_str().unwrap().to_string();
    let fingerprint = definition["fingerprint"].as_str().unwrap().to_string();
    let cli = Command::new(env!("CARGO_BIN_EXE_runui"))
        .args(["commands", "-C", dir.to_str().unwrap(), "--query", "source:make build", "--json"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .output()
        .unwrap();
    assert!(cli.status.success());
    let catalog: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(catalog["commands"][0]["id"], definition_id);
    assert_eq!(catalog["commands"][0]["fingerprint"], fingerprint);
    let conflict = client.tool(42, "start_run", json!({"id":definition_id,"command":"echo conflict"}));
    assert_eq!(conflict["isError"], true);
    fs::write(dir.join("Makefile"), "build: ## Build the app\n\techo changed\n").unwrap();
    let stale = client.tool(43, "start_run", json!({"id":definition_id,"fingerprint":fingerprint}));
    assert_eq!(stale["isError"], true);
    let native = client.tool(44, "start_run", json!({"id":definition_id}));
    assert_eq!(native["isError"], false, "{native}");
    assert_eq!(native["structuredContent"]["definition_id"], definition_id);
    assert_eq!(native["structuredContent"]["agent"], "test");

    let started = client.tool(5, "start_run", json!({"command": "echo MCP_READY; sleep 5", "name": "mcp test"}));
    assert_eq!(started["isError"], false, "{started}");
    assert_eq!(started["structuredContent"]["agent"], "test", "the run isn't the client's: {started}");
    let file = started["structuredContent"]["output_file"].as_str().unwrap().to_string();
    assert!(file.ends_with("/output.log") && file.contains(&id_of(&started)), "its output file is {file}");
    let id = started["structuredContent"]["id"].as_str().unwrap().to_string();
    let listed = client.tool(6, "list_runs", json!({}));
    assert!(listed["structuredContent"]["runs"].as_array().unwrap().iter().any(|run| run["id"] == id));

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = client.tool(7, "read_output", json!({"id": id, "lines": 20}));
        assert_eq!(output["isError"], false, "{output}");
        if output["structuredContent"]["output"].as_str().unwrap().contains("MCP_READY") {
            break;
        }
        assert!(Instant::now() < deadline, "run produced no output");
        std::thread::sleep(Duration::from_millis(20));
    }

    let invalid = client.tool(8, "stop_run", json!({"id": "../outside"}));
    assert_eq!(invalid["isError"], true);
    let stopped = client.tool(9, "stop_run", json!({"id": id}));
    assert_eq!(stopped["isError"], false, "{stopped}");
    let stop_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let run = client.tool(10, "get_run", json!({"id": id}));
        if run["structuredContent"]["state"] == "stopped" {
            break;
        }
        assert!(Instant::now() < stop_deadline, "run did not stop: {run}");
        std::thread::sleep(Duration::from_millis(20));
    }
    let saved = client.tool(12, "read_output", json!({"id": id}));
    assert!(saved["structuredContent"]["output"].as_str().unwrap().contains("MCP_READY"));
}
