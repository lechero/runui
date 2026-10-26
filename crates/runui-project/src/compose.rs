//! A docker compose file's services, and what runs them.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use runui_core::golang::{filepath, read_dir};

use crate::yaml::{self, Doc, MAPPING_NODE, SCALAR_NODE};
use crate::{Command, Source, is_file, quote};

/// COMPOSE_DEFAULTS are the names docker compose looks for in a folder, in
/// its order: the first one there is what docker compose alone reads.
const COMPOSE_DEFAULTS: [&str; 4] = ["compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"];

/// COMPOSE_NAME is a compose file's name: compose.yaml, docker-compose.yml,
/// and ones for a purpose, docker-compose.dev.yml.
static COMPOSE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:docker-)?compose(?:\.[A-Za-z0-9_-]+)?\.ya?ml$").expect("a regexp"));

pub(crate) fn compose_default(name: &str) -> bool {
    COMPOSE_DEFAULTS.contains(&name)
}

/// compose_files are a folder's compose files: the one docker compose
/// reads by itself first, then the others by name. An override file
/// (compose.override.yaml) isn't one of them: docker compose adds it to
/// the one it reads by itself.
pub(crate) fn compose_files(dir: &str) -> Vec<String> {
    let Ok(entries) = read_dir(Path::new(dir)) else { return Vec::new() };
    let mut others: Vec<String> = Vec::new();
    for e in entries {
        let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
        if !COMPOSE_NAME.is_match(&name) || name.contains(".override.") || !is_file(&filepath::join(&[dir, &name])) {
            continue;
        }
        others.push(name);
    }
    let first = COMPOSE_DEFAULTS.iter().find(|name| others.iter().any(|o| o == *name)).map(|n| n.to_string());
    others.sort();
    let mut out = Vec::new();
    if let Some(first) = &first {
        out.push(filepath::join(&[dir, first]));
    }
    for name in &others {
        if Some(name) != first.as_ref() {
            out.push(filepath::join(&[dir, name]));
        }
    }
    out
}

/// read_compose is what a compose file runs: every service up (its logs
/// following, the way runui shows output, and s stopping them all), built
/// again first, down, their state and their logs, then each service up on
/// its own with what it's made from. named runs it with -f, for a file
/// docker compose doesn't read by itself.
pub fn read_compose(path: &str, named: bool) -> (Vec<Command>, Option<anyhow::Error>) {
    read_tracked(path, named, &mut crate::discovery::Scan::new(&filepath::dir(path)))
}

pub(crate) fn read_tracked(
    path: &str,
    named: bool,
    scan: &mut crate::discovery::Scan,
) -> (Vec<Command>, Option<anyhow::Error>) {
    let data = match scan.read(path) {
        Ok(data) => data,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let (doc, cd) = match yaml::unmarshal_compose(&data) {
        Ok(d) => d,
        Err(e) => return (Vec::new(), Some(runui_core::wrap(e.clone(), format!("{}: {e}", filepath::base(path))))),
    };
    let mut base = "docker compose".to_string();
    if named {
        base += &format!(" -f {}", quote(&filepath::base(path)));
    }
    let cmd = |name: String, arguments: Vec<String>, desc: String| {
        let mut argv = vec!["docker".into(), "compose".into()];
        if named {
            argv.extend(["-f".into(), filepath::base(path)]);
        }
        argv.extend(arguments);
        Command {
            source: Source::Compose,
            run: format!("{base} {name}"),
            name,
            desc,
            argv,
            args_separator: false,
            defining_file: path.into(),
            defining_line: None,
            variant: String::new(),
            generated: true,
            aliases: Vec::new(),
            hook: false,
            requirements: Vec::new(),
            cargo: None,
        }
    };
    let mut cmds = vec![
        cmd("up".into(), vec!["up".into()], "Start every service; their logs follow here, and s stops them".into()),
        cmd(
            "up --build".into(),
            vec!["up".into(), "--build".into()],
            "Build the images again, then start every service".into(),
        ),
        cmd("down".into(), vec!["down".into()], "Stop the services and remove their containers and networks".into()),
        cmd("ps".into(), vec!["ps".into()], "The services' containers, and how they are".into()),
        cmd(
            "logs -f".into(),
            vec!["logs".into(), "-f".into()],
            "Follow the services' logs, however they were started".into(),
        ),
    ];
    for (name, svc) in doc.pairs(cd.services) {
        let mut c = cmd(
            format!("up {}", quote(&name)),
            vec!["up".into(), name.clone()],
            format!("Start {name} and what it depends on"),
        );
        let from = service_from(&doc, svc);
        if !from.is_empty() {
            c.desc += &format!(": {from}");
        }
        cmds.push(c);
    }
    (cmds, None)
}

/// service_from says what a service is made from: its image, or what it's
/// built from.
fn service_from(doc: &Doc, n: yaml::NodeRef) -> String {
    if doc.get(n).kind != MAPPING_NODE {
        return String::new();
    }
    let (svc, err) = doc.decode_service(n);
    if err.is_err() {
        return String::new();
    }
    let mut built = String::new();
    if let Some(b) = svc.build {
        match doc.get(b).kind {
            SCALAR_NODE => built = doc.get(b).value.clone(),
            MAPPING_NODE => {
                let (build, _) = doc.decode_build(b);
                built = or_else(&build.context, ".");
            }
            _ => {}
        }
    }
    if !built.is_empty() {
        return format!("built from {built}");
    }
    if !svc.image.is_empty() {
        return format!("image {}", svc.image);
    }
    String::new()
}

/// or_else is s, or otherwise when s is empty.
fn or_else(s: &str, otherwise: &str) -> String {
    if s.is_empty() { otherwise.to_string() } else { s.to_string() }
}
