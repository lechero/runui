//! Native checks are explicit and ephemeral; they never decorate a cached
//! snapshot with readiness from another environment or run project scripts.

use std::process::Command;
use std::sync::atomic::AtomicBool;

use runui_runs::inspection::{self, End, Limits};
use serde::Serialize;

use crate::{Entry, Kind, Recipe};

mod cargo;
mod environment;
pub use cargo::CargoMetadata;
pub use environment::EnvironmentReport;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo: Option<CargoMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentReport>,
}

/// Only provider-owned version/metadata probes run here. Supported mise wrappers
/// resolve installed tools in safe mode; other wrappers stay explicitly unchecked.
pub fn check(entry: &Entry, cancel: &AtomicBool) -> Report {
    let skip = |message: &str| Report {
        checked: Vec::new(),
        skipped: Some(message.into()),
        ready: false,
        cargo: None,
        environment: None,
    };
    if !matches!(entry.source.as_str(), "composer" | "cargo") || !matches!(entry.kind, Kind::Command | Kind::Hook) {
        return skip("Native checks support Composer scripts and generated Cargo definitions.");
    }
    if entry.cargo_alias.is_some() {
        return skip(
            "Native inspection does not execute Cargo aliases. Inspect the effective declaration and its config origins; Cargo owns expansion when you run it.",
        );
    }
    let Recipe::Tool { argv, shell, .. } = &entry.recipe else {
        return skip("No native tool recipe.");
    };
    let mut report = Report { checked: Vec::new(), skipped: None, ready: false, cargo: None, environment: None };
    let native_environment = if !shell.is_empty() && shell != &["/bin/sh", "-c"] {
        match environment::resolve(entry, &mut report, cancel) {
            Ok(environment) => environment,
            Err(error) => {
                report.skipped = Some(error.to_string());
                return report;
            }
        }
    } else {
        environment::NativeEnvironment::default()
    };
    let mut selected = entry.clone();
    if report.environment.is_some() {
        for requirement in &mut selected.requirements {
            requirement.resolved_path = native_environment.resolve(entry, &requirement.program);
            let Some(path) = &requirement.resolved_path else {
                report.skipped = Some(format!(
                    "{} is unavailable in the safe installed-tool environment; static discovery was kept",
                    requirement.program
                ));
                return report;
            };
            if matches!(requirement.program.as_str(), "php" | "cargo" | "rustc")
                && let Err(error) = environment::binary(path)
            {
                report.skipped = Some(format!("{}: {error}", requirement.program));
                return report;
            }
        }
    }
    let entry = &selected;
    if entry.source == "cargo" {
        return cargo::check_in(entry, cancel, report, &native_environment);
    }
    let Some(position) = argv.iter().position(|arg| arg == "run-script") else {
        return skip("Invalid Composer recipe.");
    };
    let mut composer = argv[..position].to_vec();
    for requirement in &entry.requirements {
        if let Some(path) = &requirement.resolved_path
            && let Some(part) = composer.iter_mut().find(|arg| **arg == requirement.program)
        {
            *part = path.clone();
        }
    }
    composer.extend(["--no-plugins", "--no-scripts", "--no-ansi", "--no-interaction", "--version"].map(str::to_string));
    let php = entry
        .requirements
        .iter()
        .find(|tool| tool.program == "php")
        .and_then(|tool| tool.resolved_path.clone())
        .unwrap_or_else(|| "php".into());
    for (tool, args) in [("php", vec![php, "--version".into()]), ("composer", composer)] {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            report.skipped = Some("Native inspection cancelled.".into());
            return report;
        }
        let mut command = Command::new(&args[0]);
        command.args(&args[1..]).current_dir(&entry.directory).env("COMPOSER_DISABLE_NETWORK", "1");
        native_environment.apply(&mut command);
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
        report.checked.push(Probe { tool: tool.into(), argv: args, output, error, supported });
    }
    report.ready = report.checked.iter().all(|probe| probe.supported);
    report
}
