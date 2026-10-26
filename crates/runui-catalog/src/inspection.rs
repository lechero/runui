//! Native checks are explicit and ephemeral; they never decorate a cached
//! snapshot with readiness from another environment or run project scripts.

use std::process::Command;
use std::sync::atomic::AtomicBool;

use runui_runs::inspection::{self, End, Limits};
use serde::Serialize;

use crate::{Entry, Kind, Recipe};

#[derive(Debug, Serialize)]
pub struct Probe {
    pub tool: String,
    pub argv: Vec<String>,
    pub output: Option<inspection::Output>,
    pub error: Option<String>,
    pub supported: bool,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub checked: Vec<Probe>,
    pub skipped: Option<String>,
    pub ready: bool,
}

/// Only provider-owned version probes run here. A shell wrapper may provision
/// tools or evaluate project config; until it has an offline contract we leave
/// its availability unknown rather than silently bypassing it.
pub fn check(entry: &Entry, cancel: &AtomicBool) -> Report {
    let skip = |message: &str| Report { checked: Vec::new(), skipped: Some(message.into()), ready: false };
    if entry.source != "composer" || !matches!(entry.kind, Kind::Command | Kind::Hook) {
        return skip("Native checks currently support Composer script definitions.");
    }
    let Recipe::Tool { argv, shell, .. } = &entry.recipe else {
        return skip("No native tool recipe.");
    };
    if !shell.is_empty() && shell != &["/bin/sh", "-c"] {
        return skip(
            "Project shell wrapper remains unverified: native inspection does not activate wrappers or provision tools.",
        );
    }
    let Some(position) = argv.iter().position(|arg| arg == "run-script") else {
        return skip("Invalid Composer recipe.");
    };
    let mut composer = argv[..position].to_vec();
    for requirement in &entry.requirements {
        if let Some(path) = &requirement.resolved_path
            && let Some(part) = composer.iter_mut().skip(2).find(|arg| **arg == requirement.program)
        {
            *part = path.clone();
        }
    }
    composer.extend(["--no-plugins", "--no-scripts", "--no-ansi", "--no-interaction", "--version"].map(str::to_string));
    let mut checked = Vec::new();
    let php = entry
        .requirements
        .iter()
        .find(|tool| tool.program == "php")
        .and_then(|tool| tool.resolved_path.clone())
        .unwrap_or_else(|| "php".into());
    for (tool, args) in [("php", vec![php, "--version".into()]), ("composer", composer)] {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Report { checked, skipped: Some("Native inspection cancelled.".into()), ready: false };
        }
        let mut command = Command::new(&args[0]);
        command.args(&args[1..]).current_dir(&entry.directory).env("COMPOSER_DISABLE_NETWORK", "1");
        let (output, error) = match inspection::run(&mut command, Limits::default(), cancel) {
            Ok(output) => (Some(output), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let supported = output.as_ref().is_some_and(|output| {
            output.end == End::Exited
                && output.code == Some(0)
                && output.stdout.lines().chain(output.stderr.lines()).any(|line| {
                    let prefix = if tool == "php" { "PHP " } else { "Composer version " };
                    let version =
                        line.strip_prefix(prefix).and_then(|line| line.split_whitespace().next()).unwrap_or_default();
                    if tool == "composer" {
                        version
                            .strip_prefix("2.")
                            .and_then(|version| version.split('.').next())
                            .and_then(|minor| minor.parse::<u32>().ok())
                            .is_some_and(|minor| minor >= 2)
                    } else {
                        version
                            .split('.')
                            .next()
                            .and_then(|major| major.parse::<u32>().ok())
                            .is_some_and(|major| major >= 5)
                    }
                })
        });
        checked.push(Probe { tool: tool.into(), argv: args, output, error, supported });
    }
    Report { ready: checked.iter().all(|probe| probe.supported), checked, skipped: None }
}
