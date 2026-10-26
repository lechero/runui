//! The agent CLIs runui's MCP server can be installed for: Claude Code,
//! Codex, GitHub Copilot, Gemini CLI, Qwen Code and Amp. Which of them are
//! on the PATH, whether their settings start runui's server, and their own
//! `mcp add` and `mcp remove`, which put it there and take it out again, so
//! each agent writes its settings itself. The server is installed for the
//! user, everywhere, as `runui mcp` with no folder: an agent starts it in
//! the folder it works in, and it serves that folder's project.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

/// SERVER is the name runui's MCP server is installed under.
pub const SERVER: &str = "runui";

/// TIMEOUT is how long an agent's own command gets to add or remove it.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Settings is where an agent keeps its MCP servers: under a key of a JSON
/// file, or a table of a TOML one.
#[derive(Clone, Copy, Debug)]
enum Settings {
    Json(&'static str),
    Toml(&'static str),
}

/// Spec is an agent CLI runui knows: its command, the file it keeps its MCP
/// servers in, and its arguments that add and remove one.
#[derive(Debug)]
pub struct Spec {
    pub name: &'static str,
    /// its command, which tells it apart
    pub bin: &'static str,
    /// a variable naming the folder its file is in; "" for none
    var: &'static str,
    /// that folder under the home folder, without the variable
    dir: &'static str,
    /// the file, in that folder
    file: &'static str,
    settings: Settings,
    /// its arguments that add a server, before the server's name
    add: &'static [&'static str],
    /// a -- between the server's name and the command it starts
    dashes: bool,
    /// its arguments that remove a server, before the server's name
    remove: &'static [&'static str],
}

/// SPECS are the agents runui knows, in the order it lists them. Each adds
/// the server for the user, not for one project.
pub static SPECS: &[Spec] = &[
    Spec {
        name: "Claude Code",
        bin: "claude",
        var: "CLAUDE_CONFIG_DIR",
        dir: "",
        file: ".claude.json",
        settings: Settings::Json("mcpServers"),
        add: &["mcp", "add", "--scope", "user"],
        dashes: true,
        remove: &["mcp", "remove", "--scope", "user"],
    },
    Spec {
        name: "Codex",
        bin: "codex",
        var: "CODEX_HOME",
        dir: ".codex",
        file: "config.toml",
        settings: Settings::Toml("mcp_servers"),
        add: &["mcp", "add"],
        dashes: true,
        remove: &["mcp", "remove"],
    },
    Spec {
        name: "GitHub Copilot",
        bin: "copilot",
        var: "COPILOT_HOME",
        dir: ".copilot",
        file: "mcp-config.json",
        settings: Settings::Json("mcpServers"),
        add: &["mcp", "add"],
        dashes: true,
        remove: &["mcp", "remove"],
    },
    Spec {
        name: "Gemini CLI",
        bin: "gemini",
        var: "",
        dir: ".gemini",
        file: "settings.json",
        settings: Settings::Json("mcpServers"),
        add: &["mcp", "add", "--scope", "user"],
        dashes: false,
        remove: &["mcp", "remove", "--scope", "user"],
    },
    Spec {
        name: "Qwen Code",
        bin: "qwen",
        var: "",
        dir: ".qwen",
        file: "settings.json",
        settings: Settings::Json("mcpServers"),
        add: &["mcp", "add", "--scope", "user"],
        dashes: false,
        remove: &["mcp", "remove", "--scope", "user"],
    },
    Spec {
        name: "Amp",
        bin: "amp",
        var: "XDG_CONFIG_HOME",
        dir: ".config",
        file: "amp/settings.json",
        settings: Settings::Json("amp.mcpServers"),
        add: &["mcp", "add"],
        dashes: true,
        remove: &["mcp", "remove"],
    },
];

/// Agents is where runui looks for agent CLIs: the home folder, the PATH,
/// the variables that move an agent's settings, and the runui an agent is
/// to start.
#[derive(Clone, Debug, Default)]
pub struct Agents {
    pub home: String,
    /// the folders commands are looked for in, as PATH has them
    pub path: String,
    /// the variables, of the ones the agents read, that are set
    pub vars: HashMap<String, String>,
    /// the runui an agent starts: an absolute path
    pub runui: String,
}

impl Agents {
    /// current is the agents as this process finds them: its PATH and
    /// variables, and runui as the PATH finds it, which outlasts an
    /// upgrade, else this very runui.
    pub fn current(home: &str) -> Agents {
        let path = std::env::var("PATH").unwrap_or_default();
        let vars = SPECS
            .iter()
            .filter(|s| !s.var.is_empty())
            .filter_map(|s| std::env::var(s.var).ok().map(|v| (s.var.to_string(), v)))
            .collect();
        let runui = lookup("runui", &path)
            .or_else(|| std::env::current_exe().ok().and_then(|p| p.to_str().map(str::to_string)))
            .unwrap_or_else(|| "runui".into());
        Agents { home: home.to_string(), path, vars, runui }
    }

    /// list is every agent runui knows, the ones on the PATH first.
    pub fn list(&self) -> Vec<Status> {
        let mut list: Vec<Status> = SPECS.iter().map(|s| self.status(s)).collect();
        list.sort_by_key(|s| !s.found());
        list
    }

    /// status is where an agent is, and runui's server in its settings.
    pub fn status(&self, spec: &'static Spec) -> Status {
        let file = self.file(spec);
        let (server, err) = match read_server(&file, spec.settings) {
            Ok(server) => (server, String::new()),
            Err(e) => (None, format!("{e:#}")),
        };
        Status { spec, bin: lookup(spec.bin, &self.path).unwrap_or_default(), file, server, err }
    }

    /// file is the file an agent keeps its MCP servers in.
    fn file(&self, spec: &Spec) -> PathBuf {
        let dir = match self.vars.get(spec.var) {
            Some(v) if !spec.var.is_empty() && !v.is_empty() => PathBuf::from(v),
            _ => Path::new(&self.home).join(spec.dir),
        };
        dir.join(spec.file)
    }

    /// install is the agent's command that adds runui's server, as it's
    /// typed: `runui mcp`, with no folder.
    pub fn install(&self, spec: &Spec) -> Vec<String> {
        let mut argv: Vec<String> =
            std::iter::once(spec.bin).chain(spec.add.iter().copied()).map(String::from).collect();
        argv.push(SERVER.into());
        if spec.dashes {
            argv.push("--".into());
        }
        argv.extend([self.runui.clone(), "mcp".into()]);
        argv
    }

    /// uninstall is the agent's command that removes runui's server.
    pub fn uninstall(&self, spec: &Spec) -> Vec<String> {
        std::iter::once(spec.bin).chain(spec.remove.iter().copied()).chain([SERVER]).map(String::from).collect()
    }

    /// change runs the agent's own command that installs runui's server, or
    /// removes it, in the home folder, and says what it printed: the error
    /// is that too when it fails.
    pub fn change(&self, st: &Status, install: bool) -> Result<String> {
        if !st.found() {
            return Err(anyhow!("{} isn't on the PATH", st.spec.bin));
        }
        let mut argv = if install { self.install(st.spec) } else { self.uninstall(st.spec) };
        argv[0] = st.bin.clone();
        execute(&argv, &self.home, TIMEOUT)
    }
}

/// Status is an agent as runui finds it.
#[derive(Clone, Debug)]
pub struct Status {
    pub spec: &'static Spec,
    /// where its command is on the PATH; "" when it isn't there
    pub bin: String,
    /// the file it keeps its MCP servers in
    pub file: PathBuf,
    /// runui's server there, when it has one
    pub server: Option<Server>,
    /// a problem reading the file; one that isn't there yet is none
    pub err: String,
}

impl Status {
    /// found says the agent's command is on the PATH.
    pub fn found(&self) -> bool {
        !self.bin.is_empty()
    }

    /// current says runui's server is there as runui installs it: runui
    /// mcp, the runui given, with no folder of its own.
    pub fn current(&self, runui: &str) -> bool {
        self.server.as_ref().is_some_and(|s| s.args == ["mcp"] && same_file(&s.command, runui))
    }
}

/// Server is an MCP server as an agent's settings have it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Server {
    /// the command it starts, and its arguments
    pub command: String,
    pub args: Vec<String>,
    /// where it's reached, for one the agent reaches over HTTP
    pub url: String,
}

impl Server {
    /// line is the server as a command line, or its address.
    pub fn line(&self) -> String {
        if self.command.is_empty() {
            return self.url.clone();
        }
        line(&std::iter::once(self.command.clone()).chain(self.args.iter().cloned()).collect::<Vec<_>>())
    }
}

/// read_server is runui's server in an agent's settings file: None when the
/// file or the server isn't there.
fn read_server(file: &Path, settings: Settings) -> Result<Option<Server>> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    match settings {
        Settings::Json(key) => {
            let v: Value = serde_json::from_str(&text)
                .or_else(|_| serde_json::from_str(&strip_comments(&text)))
                .with_context(|| format!("reading {}", file.display()))?;
            let Some(entry) = v.get(key).and_then(|servers| servers.get(SERVER)) else { return Ok(None) };
            let text = |k: &str| entry.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let args = entry.get("args").and_then(Value::as_array).map(|a| strings(a.iter().map(Value::as_str)));
            let url = if text("url").is_empty() { text("httpUrl") } else { text("url") };
            Ok(Some(Server { command: text("command"), args: args.unwrap_or_default(), url }))
        }
        Settings::Toml(key) => {
            let t: toml::Table = text.parse().with_context(|| format!("reading {}", file.display()))?;
            let Some(entry) = t.get(key).and_then(|servers| servers.get(SERVER)) else { return Ok(None) };
            let text = |k: &str| entry.get(k).and_then(toml::Value::as_str).unwrap_or_default().to_string();
            let args =
                entry.get("args").and_then(toml::Value::as_array).map(|a| strings(a.iter().map(toml::Value::as_str)));
            Ok(Some(Server { command: text("command"), args: args.unwrap_or_default(), url: text("url") }))
        }
    }
}

fn strings<'a>(items: impl Iterator<Item = Option<&'a str>>) -> Vec<String> {
    items.map(|s| s.unwrap_or_default().to_string()).collect()
}

/// strip_comments takes the // and /* */ comments out of JSON that has
/// them, as Gemini CLI's settings may, leaving strings as they are.
fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        let next = chars.clone().next();
        match (c, next) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push(n);
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// lookup is a command's path as a shell finds it on the PATH: the first
/// folder with an executable file of that name. A folder that isn't
/// absolute is passed over, as it would be the folder runui is in.
fn lookup(bin: &str, path: &str) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    path.split(':')
        .filter(|d| d.starts_with('/'))
        .map(|d| Path::new(d).join(bin))
        .find(|p| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
        .and_then(|p| p.to_str().map(str::to_string))
}

/// same_file says two paths are one file, through a symlink too.
fn same_file(a: &str, b: &str) -> bool {
    a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// line is a command as it's typed in sh: each argument quoted when it has
/// to be.
pub fn line(argv: &[String]) -> String {
    let safe = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-~".contains(c));
    argv.iter()
        .map(|a| if safe(a) { a.clone() } else { format!("'{}'", a.replace('\'', r"'\''")) })
        .collect::<Vec<_>>()
        .join(" ")
}

/// execute runs a command in a folder, with nothing to read, and gives back
/// what it printed, its output then its errors. It fails, saying what it
/// printed, when the command fails or doesn't end in time; it's killed then.
fn execute(argv: &[String], dir: &str, timeout: Duration) -> Result<String> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {}", argv[0]))?;
    let (tx, rx) = mpsc::channel();
    let streams: [Option<Box<dyn Read + Send>>; 2] = [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
    ];
    for (i, stream) in streams.into_iter().enumerate() {
        let (tx, Some(mut stream)) = (tx.clone(), stream) else { continue };
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf);
            let _ = tx.send((i, buf));
        });
    }
    drop(tx);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!("{} didn't end in {} s, so it was stopped", line(argv), timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // what it printed; a process it left behind holding the streams open
    // isn't waited for long
    let mut printed = [String::new(), String::new()];
    let until = Instant::now() + Duration::from_secs(2);
    while let Ok((i, buf)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        printed[i] = String::from_utf8_lossy(&buf).trim().to_string();
    }
    let out = printed.iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join("\n");
    if status.success() {
        return Ok(out);
    }
    if out.is_empty() {
        return Err(anyhow!("{} ended badly: {status}", line(argv)));
    }
    Err(anyhow!(out))
}

#[cfg(test)]
mod tests;
