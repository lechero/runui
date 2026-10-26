//! A local MCP stdio server for the commands and runs runui already keeps.

use std::fs;
use std::io::{BufRead, Write};
use std::path::{Component, Path};

use anyhow::{Context, Result, anyhow};
use runui_project as project;
use runui_runs::{Output, Run, State};
use runui_tea::ansi;
use serde::Deserialize;
use serde_json::{Value, json};

use super::Workspace;

const PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_LINES: usize = 100;
const MAX_LINES: usize = 500;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Pending,
    AwaitingInitialized,
    Ready,
}

/// AGENT_CHARS is how much of the client's name a run keeps.
const AGENT_CHARS: usize = 40;

/// Session is what the server knows of its client: how far the handshake
/// has got, and the name its runs are written down with.
struct Session {
    phase: Phase,
    agent: String,
}

/// serve reads one JSON-RPC message per line and writes only protocol
/// messages to stdout. The client owns this process and closes stdin to end it.
pub(super) fn serve(w: &Workspace, mut input: impl BufRead, out: &mut dyn Write) -> Result<()> {
    let mut session = Session { phase: Phase::Pending, agent: String::new() };
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line).context("reading an MCP request")? == 0 {
            break;
        }
        let response = match serde_json::from_slice(&line) {
            Ok(request) => handle(w, &mut session, request),
            Err(_) => Some(rpc_error(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut *out, &response)?;
            out.write_all(b"\n")?;
            out.flush()?;
        }
    }
    Ok(())
}

/// agent_name is the name a client gives itself in initialize, as a run
/// keeps it: on one line, its first AGENT_CHARS characters; "mcp" when it
/// gives none.
fn agent_name(params: &Value) -> String {
    let name = params["clientInfo"]["name"].as_str().unwrap_or_default();
    let name: String =
        name.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(AGENT_CHARS).collect();
    if name.is_empty() { "mcp".to_string() } else { name }
}

fn handle(w: &Workspace, session: &mut Session, request: Value) -> Option<Value> {
    let phase = &mut session.phase;
    let Some(object) = request.as_object() else {
        return Some(rpc_error(Value::Null, -32600, "Invalid Request"));
    };
    let id = object.get("id");
    let method = object.get("method").and_then(Value::as_str);
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || method.is_none()
        || id.is_some_and(|id| !id.is_string() && !id.is_number())
    {
        return Some(rpc_error(Value::Null, -32600, "Invalid Request"));
    }
    let method = method?;
    if method == "notifications/initialized" && id.is_none() {
        if *phase == Phase::AwaitingInitialized {
            *phase = Phase::Ready;
        }
        return None;
    }
    let id = id?.clone(); // other notifications need no response
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => {
            if params.get("protocolVersion").and_then(Value::as_str).is_none() {
                return Some(rpc_error(id, -32602, "Missing protocolVersion"));
            }
            *phase = Phase::AwaitingInitialized;
            session.agent = agent_name(&params);
            let requested = params["protocolVersion"].as_str().unwrap_or_default();
            let version = if SUPPORTED_VERSIONS.contains(&requested) { requested } else { PROTOCOL_VERSION };
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "runui", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Runui starts shell commands under persistent supervisors. Use list_commands to discover commands, start_run to launch one, and its returned ID to read output or stop it."
            })
        }
        "ping" => json!({}),
        "server/discover" => return Some(rpc_error(id, -32601, "Method not found")),
        _ if *phase != Phase::Ready => return Some(rpc_error(id, -32000, "Server not initialized")),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "Missing tool name"));
            };
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match call(w, &session.agent, name, args) {
                Ok(data) => tool_result(data),
                Err(e) => json!({"content": [{"type": "text", "text": format!("{e:#}")}], "isError": true}),
            }
        }
        _ => return Some(rpc_error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_result(data: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": data.to_string()}],
        "structuredContent": data,
        "isError": false
    })
}

fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "list_commands",
            "description": "List the selected project's discovered tasks, scripts, targets, services, builds, and saved favorites. Returns exact shell commands and working directories for start_run.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"readOnlyHint": true}
        }),
        json!({
            "name": "start_run",
            "description": "Start a shell command under runui's persistent supervisor. It keeps running after this MCP connection closes, and runui lists it in its Agents tab under this client's name. The command is passed to sh -c (through mise when available).",
            "inputSchema": {"type": "object", "properties": {
                "command": {"type": "string", "description": "The exact shell command to run."},
                "directory": {"type": "string", "description": "Working directory, absolute or relative to the selected project; defaults to the project directory."},
                "name": {"type": "string", "description": "Optional label shown in runui."}
            }, "required": ["command"], "additionalProperties": false}
        }),
        json!({
            "name": "list_runs",
            "description": "List persisted runs from all folders, newest first. Optionally filter to one exact working directory.",
            "inputSchema": {"type": "object", "properties": {
                "directory": {"type": "string", "description": "Optional exact working directory, absolute or relative to the selected project."}
            }, "additionalProperties": false},
            "annotations": {"readOnlyHint": true}
        }),
        json!({
            "name": "get_run",
            "description": "Get the current state and metadata of a run by its ID.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"], "additionalProperties": false},
            "annotations": {"readOnlyHint": true}
        }),
        json!({
            "name": "read_output",
            "description": "Read the latest plain-text output of a run, with the current run state. Returns at most 500 lines and 64 KiB per call; poll again to follow a live run.",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "string"},
                "lines": {"type": "integer", "minimum": 1, "maximum": MAX_LINES, "default": DEFAULT_LINES}
            }, "required": ["id"], "additionalProperties": false},
            "annotations": {"readOnlyHint": true}
        }),
        json!({
            "name": "stop_run",
            "description": "Ask a run's supervisor to stop its process group. This returns after the stop signal is sent; use get_run to wait for the final state.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"], "additionalProperties": false},
            "annotations": {"destructiveHint": true}
        }),
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartArgs {
    command: String,
    #[serde(default)]
    directory: Option<String>,
    #[serde(default)]
    name: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    directory: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdArgs {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputArgs {
    id: String,
    #[serde(default = "default_lines")]
    lines: usize,
}

fn default_lines() -> usize {
    DEFAULT_LINES
}

fn call(w: &Workspace, agent: &str, name: &str, args: Value) -> Result<Value> {
    match name {
        "list_commands" => {
            let _: EmptyArgs = serde_json::from_value(args)?;
            list_commands(w)
        }
        "start_run" => start_run(w, agent, serde_json::from_value(args)?),
        "list_runs" => list_runs(w, serde_json::from_value(args)?),
        "get_run" => {
            let args: IdArgs = serde_json::from_value(args)?;
            let run = get_run(w, &args.id)?;
            Ok(run_data(&run))
        }
        "read_output" => read_output(w, serde_json::from_value(args)?),
        "stop_run" => {
            let args: IdArgs = serde_json::from_value(args)?;
            let run = get_run(w, &args.id)?;
            w.runs()?.stop(&run)?;
            Ok(json!({"id": run.id, "stop_requested": true}))
        }
        _ => Err(anyhow!("unknown tool {name}")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

fn list_commands(w: &Workspace) -> Result<Value> {
    let mut commands = Vec::new();
    for favorite in w.favorites()?.for_dir(&w.dir) {
        commands.push(json!({
            "kind": "favorite",
            "source": favorite.source,
            "name": favorite.label(),
            "command": favorite.command,
            "directory": Path::new(&w.dir).join(&favorite.dir).to_string_lossy(),
        }));
    }
    let mut errors = Vec::new();
    let discovery = project::discover(&w.dir, &w.repo);
    let complete = discovery.complete();
    let diagnostics: Vec<_> = discovery.diagnostics.iter().map(|problem| json!({
        "file": problem.path, "source": problem.source.map(|source| source.as_str()), "message": problem.message,
    })).collect();
    for file in discovery.files {
        if let Some(error) = file.err {
            errors.push(json!({"file": file.path, "error": error}));
        }
        for command in file.commands {
            commands.push(json!({
                "kind": "project",
                "source": command.source.as_str(),
                "name": command.name,
                "command": command.run,
                "description": command.desc,
                "directory": file.dir,
                "file": file.path,
            }));
        }
    }
    Ok(
        json!({"project_directory": w.dir, "commands": commands, "errors": errors, "complete": complete, "diagnostics": diagnostics}),
    )
}

fn start_run(w: &Workspace, agent: &str, args: StartArgs) -> Result<Value> {
    if args.command.trim().is_empty() {
        return Err(anyhow!("command must not be empty"));
    }
    let dir = directory(w, args.directory.as_deref())?;
    let (run, error) = w.runs()?.start_by(&dir, &args.name, &args.command, agent);
    if let Some(error) = error {
        return Err(error);
    }
    Ok(run_data(&run))
}

fn list_runs(w: &Workspace, args: ListArgs) -> Result<Value> {
    let filter = args.directory.as_deref().map(|dir| directory(w, Some(dir))).transpose()?;
    let runs = w
        .runs()?
        .list()?
        .iter()
        .filter(|run| filter.as_ref().is_none_or(|dir| run.dir == *dir))
        .map(run_data)
        .collect::<Vec<_>>();
    Ok(json!({"runs": runs}))
}

fn read_output(w: &Workspace, args: OutputArgs) -> Result<Value> {
    if !(1..=MAX_LINES).contains(&args.lines) {
        return Err(anyhow!("lines must be between 1 and {MAX_LINES}"));
    }
    let run = get_run(w, &args.id)?;
    let path = run.output();
    let mut output = Output::new(&path, args.lines + 1);
    output.read().with_context(|| format!("reading output for run {}", run.id))?;
    let mut lines = output.lines();
    let mut truncated = fs::metadata(&path)?.len() > 512 * 1024 || lines.len() > args.lines;
    if lines.len() > args.lines {
        lines.drain(..lines.len() - args.lines);
    }
    let mut text = ansi::strip(&lines.join("\n"));
    if text.len() > MAX_OUTPUT_BYTES {
        let mut from = text.len() - MAX_OUTPUT_BYTES;
        while !text.is_char_boundary(from) {
            from += 1;
        }
        text = text[from..].to_string();
        truncated = true;
    }
    Ok(json!({"id": run.id, "state": state_name(run.state()), "output": text, "truncated": truncated}))
}

fn get_run(w: &Workspace, id: &str) -> Result<Run> {
    let path = Path::new(id);
    if id.is_empty()
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(anyhow!("invalid run ID"));
    }
    w.runs()?.get(id).with_context(|| format!("finding run {id}"))
}

fn directory(w: &Workspace, input: Option<&str>) -> Result<String> {
    let input = input.filter(|s| !s.is_empty()).unwrap_or(&w.dir);
    let path = Path::new(input);
    let path = if path.is_absolute() { path.to_path_buf() } else { Path::new(&w.dir).join(path) };
    let path = path.canonicalize().with_context(|| format!("finding directory {}", path.display()))?;
    if !path.is_dir() {
        return Err(anyhow!("{} is not a directory", path.display()));
    }
    path.to_str().map(str::to_string).ok_or_else(|| anyhow!("directory path isn't UTF-8"))
}

fn run_data(run: &Run) -> Value {
    json!({
        "id": run.id,
        "directory": run.dir,
        "name": run.name,
        "command": run.command,
        "state": state_name(run.state()),
        "started": run.started.to_rfc3339(),
        "ended": run.ended.map(|time| time.to_rfc3339()),
        "exit_code": run.exit,
        "signal": run.signal,
        "error": run.error,
        "pid": run.pid,
        "agent": run.agent,
        // everything it printed, as it prints it: for a client that follows
        // it from where it got to
        "output_file": run.output(),
    })
}

fn state_name(state: State) -> &'static str {
    match state {
        State::Starting => "starting",
        State::Running => "running",
        State::Exited => "exited",
        State::Stopped => "stopped",
        State::Lost => "lost",
    }
}
