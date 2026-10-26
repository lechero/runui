use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::*;

/// World is a home folder and a folder on the PATH, both a test's own.
struct World {
    home: String,
    bin: String,
    _root: tempfile::TempDir,
}

fn new_world() -> World {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let bin = root.path().join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    World { home: home.to_str().unwrap().to_string(), bin: bin.to_str().unwrap().to_string(), _root: root }
}

impl World {
    fn agents(&self) -> Agents {
        Agents { home: self.home.clone(), path: self.bin.clone(), vars: HashMap::new(), runui: "/opt/runui".into() }
    }

    fn write(&self, rel: &str, body: &str) {
        let path = Path::new(&self.home).join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    /// script puts an executable on the PATH.
    fn script(&self, name: &str, body: &str) {
        let path = Path::new(&self.bin).join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn spec(bin: &str) -> &'static Spec {
    SPECS.iter().find(|s| s.bin == bin).unwrap()
}

#[test]
fn settings_are_read_where_each_agent_keeps_them() {
    let w = new_world();
    w.write(
        ".claude.json",
        r#"{"projects":{},"mcpServers":{"runui":{"type":"stdio","command":"/opt/runui","args":["mcp"]}}}"#,
    );
    w.write(
        ".codex/config.toml",
        "model = \"o4\"\n\n[mcp_servers.runui]\ncommand = \"/opt/runui\"\nargs = [\"mcp\", \"-C\", \"/p\"]\n",
    );
    w.write(".copilot/mcp-config.json", r#"{"mcpServers":{"other":{"command":"x"}}}"#);
    w.write(
        ".gemini/settings.json",
        "{\n  // a comment, as Gemini allows\n  \"mcpServers\": {\"runui\": {\"httpUrl\": \"http://a/*b*/\"}} /* and another */\n}\n",
    );
    w.write(".config/amp/settings.json", r#"{"amp.mcpServers":{"runui":{"command":"runui","args":["mcp"]}}}"#);
    let a = w.agents();
    let server = |bin| a.status(spec(bin)).server;
    assert_eq!(
        server("claude"),
        Some(Server { command: "/opt/runui".into(), args: vec!["mcp".into()], url: String::new() })
    );
    assert_eq!(server("codex").unwrap().line(), "/opt/runui mcp -C /p");
    assert_eq!(server("copilot"), None, "another server isn't runui's");
    assert_eq!(server("gemini").unwrap().line(), "http://a/*b*/", "a comment's marks in a string are kept");
    assert_eq!(server("amp").unwrap().line(), "runui mcp");
    let qwen = a.status(spec("qwen"));
    assert!(qwen.server.is_none() && qwen.err.is_empty(), "no file is no server, and no problem: {qwen:?}");
}

#[test]
fn a_variable_moves_an_agents_settings() {
    let w = new_world();
    w.write("elsewhere/config.toml", "[mcp_servers.runui]\ncommand = \"/opt/runui\"\nargs = [\"mcp\"]\n");
    let mut a = w.agents();
    assert!(a.status(spec("codex")).server.is_none());
    a.vars.insert("CODEX_HOME".into(), format!("{}/elsewhere", w.home));
    let st = a.status(spec("codex"));
    assert!(st.current("/opt/runui"), "CODEX_HOME wasn't followed: {st:?}");
    a.vars.insert("XDG_CONFIG_HOME".into(), format!("{}/xdg", w.home));
    assert!(a.status(spec("amp")).file.ends_with("xdg/amp/settings.json"));
}

#[test]
fn settings_that_dont_read_say_why() {
    let w = new_world();
    w.write(".claude.json", "{not json");
    w.write(".codex/config.toml", "[mcp_servers\n");
    let a = w.agents();
    for bin in ["claude", "codex"] {
        let st = a.status(spec(bin));
        assert!(st.server.is_none() && st.err.starts_with("reading "), "{bin}: {st:?}");
    }
}

#[test]
fn current_is_this_runui_serving_whatever_folder_the_agent_is_in() {
    let w = new_world();
    w.script("runui", "exit 0");
    let real = format!("{}/runui", w.bin);
    let link = format!("{}/runui-link", w.home);
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let a = w.agents();
    let mut st = a.status(spec("claude"));
    let server = |command: &str, args: &[&str]| {
        Some(Server { command: command.into(), args: args.iter().map(|s| s.to_string()).collect(), url: String::new() })
    };
    for (s, want) in [
        (server(&real, &["mcp"]), true),
        (server(&link, &["mcp"]), true),
        (server(&real, &["mcp", "-C", "/p"]), false),
        (server("/old/runui", &["mcp"]), false),
        (None, false),
    ] {
        st.server = s.clone();
        assert_eq!(st.current(&real), want, "{s:?}");
    }
}

#[test]
fn agents_on_the_path_come_first() {
    let w = new_world();
    w.script("copilot", "exit 0");
    w.script("amp", "exit 0");
    fs::write(format!("{}/codex", w.bin), "not executable").unwrap();
    let list = w.agents().list();
    let order: Vec<_> = list.iter().map(|s| (s.spec.bin, s.found())).collect();
    assert_eq!(
        order,
        [("copilot", true), ("amp", true), ("claude", false), ("codex", false), ("gemini", false), ("qwen", false)]
    );
    assert_eq!(list[0].bin, format!("{}/copilot", w.bin));
}

#[test]
fn each_agent_is_given_its_own_command() {
    let w = new_world();
    let mut a = w.agents();
    a.runui = "/Apps/my runui".into();
    let line = |argv: Vec<String>| line(&argv);
    assert_eq!(line(a.install(spec("claude"))), "claude mcp add --scope user runui -- '/Apps/my runui' mcp");
    assert_eq!(line(a.uninstall(spec("claude"))), "claude mcp remove --scope user runui");
    assert_eq!(line(a.install(spec("codex"))), "codex mcp add runui -- '/Apps/my runui' mcp");
    assert_eq!(line(a.install(spec("gemini"))), "gemini mcp add --scope user runui '/Apps/my runui' mcp");
    assert_eq!(line(a.uninstall(spec("copilot"))), "copilot mcp remove runui");
    assert_eq!(super::line(&["it's".into()]), r"'it'\''s'");
}

#[test]
fn change_runs_the_agents_own_command_from_the_path() {
    let w = new_world();
    let config = format!("{}/.claude.json", w.home);
    let log = format!("{}/args", w.home);
    // a claude that writes its settings as the real one does
    w.script(
        "claude",
        &format!(
            r#"echo "$@" >> '{log}'
pwd >> '{log}'
case "$2" in
add) printf '{{"mcpServers":{{"runui":{{"command":"%s","args":["%s"]}}}}}}' "$7" "$8" > '{config}'; echo "Added stdio MCP server runui";;
remove) echo '{{"mcpServers":{{}}}}' > '{config}'; echo "Removed MCP server runui" >&2;;
esac"#
        ),
    );
    let a = w.agents();
    let st = a.status(spec("claude"));
    assert!(st.found() && st.server.is_none());
    assert_eq!(a.change(&st, true).unwrap(), "Added stdio MCP server runui");
    let st = a.status(spec("claude"));
    assert!(st.current("/opt/runui"), "after installing: {st:?}");
    assert_eq!(a.change(&st, false).unwrap(), "Removed MCP server runui");
    assert!(a.status(spec("claude")).server.is_none());
    let canonical_home = fs::canonicalize(&w.home).unwrap();
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        format!(
            "mcp add --scope user runui -- /opt/runui mcp\n{0}\nmcp remove --scope user runui\n{0}\n",
            canonical_home.display()
        ),
        "it runs in the home folder"
    );
}

#[test]
fn a_failing_command_says_what_it_printed() {
    let w = new_world();
    w.script("codex", "echo 'Error: a server named runui exists' >&2; exit 3");
    let a = w.agents();
    let err = a.change(&a.status(spec("codex")), true).unwrap_err();
    assert_eq!(err.to_string(), "Error: a server named runui exists");
    w.script("codex", "exit 4");
    let err = a.change(&a.status(spec("codex")), true).unwrap_err();
    assert!(err.to_string().contains("ended badly"), "{err}");
    let err = a.change(&a.status(spec("gemini")), true).unwrap_err();
    assert_eq!(err.to_string(), "gemini isn't on the PATH");
}

#[test]
fn a_command_that_hangs_is_stopped() {
    let w = new_world();
    w.script("amp", "sleep 30");
    let started = Instant::now();
    let err = execute(&[format!("{}/amp", w.bin)], &w.home, Duration::from_millis(200)).unwrap_err();
    assert!(err.to_string().contains("didn't end"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(10), "it waited {:?}", started.elapsed());
}
