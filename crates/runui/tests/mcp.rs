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

#[test]
fn workspace_scope_resolves_the_same_sibling_id_as_cli_and_launches_in_its_directory() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("project");
    let web = root.join("packages/web");
    let engine = root.join("packages/a/b/engine");
    for directory in [&root.join(".git"), &web, &engine] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::write(root.join("package.json"), r#"{"workspaces":["packages/**"]}"#).unwrap();
    for directory in [&web, &engine] {
        fs::write(directory.join("package.json"), "{}").unwrap();
    }
    fs::write(engine.join("Makefile"), "check:\n\t@pwd > cwd.txt\n").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_runui"));
    command
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .current_dir(&web);
    let cli = command.args(["commands", "--scope", "workspace", "--json", "source:make check"]).output().unwrap();
    assert!(cli.status.success());
    let cli: Value = serde_json::from_slice(&cli.stdout).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_runui"))
        .args(["mcp", "-C", web.to_str().unwrap()])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut client =
        Client { input: child.stdin.take().unwrap(), output: BufReader::new(child.stdout.take().unwrap()), child };
    client.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"scope-test","version":"1"}}),
    );
    client.notify("notifications/initialized");
    let local = client.tool(2, "list_commands", json!({"query":"source:make check"}));
    assert_eq!(local["structuredContent"]["commands"], json!([]));
    let page = client.tool(3, "list_commands", json!({"scope":"workspace","query":"source:make check"}));
    let entry = &page["structuredContent"]["commands"][0];
    assert_eq!(entry["id"], cli["commands"][0]["id"]);
    assert_eq!(entry["fingerprint"], cli["commands"][0]["fingerprint"]);
    let id = entry["id"].clone();
    assert_eq!(client.tool(4, "start_run", json!({"id":id}))["isError"], true);
    let started = client.tool(5, "start_run", json!({"id":id,"scope":"workspace"}));
    assert_eq!(started["isError"], false, "{started}");
    assert_eq!(started["structuredContent"]["agent"], "scope-test");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !engine.join("cwd.txt").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(runui_catalog::same_directory(
        fs::read_to_string(engine.join("cwd.txt")).unwrap().trim(),
        engine.to_str().unwrap()
    ));
}

#[test]
fn registered_mcp_selection_matches_cli_and_revalidates_only_its_checkout() {
    let home = tempfile::tempdir().unwrap();
    let selected = home.path().join("one/app");
    let other = home.path().join("two/app");
    let favorites = home.path().join("config/runui/favorites.json");
    let registry = runui_catalog::registry::Store::beside_favorites(favorites.to_str().unwrap());
    let mut registrations = Vec::new();
    for directory in [&selected, &other] {
        fs::create_dir_all(directory.join(".git")).unwrap();
        fs::write(directory.join("Makefile"), "test:\n\t@pwd > cwd.txt\n").unwrap();
        registrations
            .push(registry.add(directory.to_str().unwrap(), home.path().to_str().unwrap(), Some("App")).unwrap());
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_runui"))
        .args(["mcp", "-C", selected.to_str().unwrap()])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut client =
        Client { input: child.stdin.take().unwrap(), output: BufReader::new(child.stdout.take().unwrap()), child };
    client.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"registry-test","version":"1"}}),
    );
    client.notify("notifications/initialized");
    let page = client.tool(2, "list_commands", json!({"scope":"registered","query":"source:make test"}));
    let commands = page["structuredContent"]["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 2, "{page}");
    let entry = commands.iter().find(|entry| entry["registry_id"] == registrations[1].id).unwrap();
    let id = entry["id"].clone();
    let fingerprint = entry["fingerprint"].clone();
    let cli = Command::new(env!("CARGO_BIN_EXE_runui"))
        .args(["commands", "-C", selected.to_str().unwrap(), "--scope", "registered", "--json", "source:make test"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .output()
        .unwrap();
    let cli: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert!(
        cli["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == id && entry["fingerprint"] == fingerprint)
    );
    assert_eq!(client.tool(3, "start_run", json!({"scope":"registered","id":id}))["isError"], true);
    fs::write(other.join("Makefile"), "test: ## Updated\n\t@pwd > cwd.txt\n").unwrap();
    assert_eq!(
        client.tool(
            4,
            "start_run",
            json!({"scope":"registered","project":registrations[1].id,"id":id,"fingerprint":fingerprint})
        )["isError"],
        true
    );
    fs::rename(&selected, home.path().join("away")).unwrap();
    let started = client.tool(5, "start_run", json!({"scope":"registered","project":registrations[1].id,"id":id}));
    assert_eq!(started["isError"], false, "{started}");
    assert_eq!(started["structuredContent"]["agent"], "registry-test");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !other.join("cwd.txt").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(runui_catalog::same_directory(
        fs::read_to_string(other.join("cwd.txt")).unwrap().trim(),
        other.to_str().unwrap()
    ));
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
