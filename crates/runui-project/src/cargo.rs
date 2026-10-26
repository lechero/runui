//! Passive Cargo manifests and targets. Cargo owns compilation and workspace
//! defaults; opening discovery never invokes it or installs a toolchain.

use std::path::{Path, PathBuf};

use runui_core::golang::filepath;
use serde::{Deserialize, Serialize};
use toml::Value;

use crate::discovery::Scan;
use crate::{Command, Source, canonical_path, quote};

mod aliases;
mod targets;
mod workspace;
pub use aliases::{
    CargoAlias, CargoAliasContext, CargoAliasEnvironment, CargoAliasOrigin, CargoAliasValue, discover_aliases,
};
use targets::{features, read_targets};
pub(crate) use workspace::{Workspace, declaration, enclosing};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CargoScope {
    Package,
    DefaultMembers,
    AllMembers,
}

impl CargoScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::DefaultMembers => "default_members",
            Self::AllMembers => "all_members",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::DefaultMembers => "default members",
            Self::AllMembers => "all members",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Library,
    Binary,
    Example,
    Test,
    Benchmark,
}

impl TargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Binary => "binary",
            Self::Example => "example",
            Self::Test => "test",
            Self::Benchmark => "benchmark",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoTarget {
    pub kind: TargetKind,
    pub name: String,
    pub source_path: String,
    pub crate_types: Vec<String>,
    pub required_features: Vec<String>,
    pub runnable: bool,
    pub default_run: bool,
}

/// Manifest context is separate from the execution cwd chosen by the catalog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoDetails {
    pub manifest_directory: String,
    pub workspace_root: Option<String>,
    pub package: Option<String>,
    pub scope: CargoScope,
    pub default_member_patterns: Option<Vec<String>>,
    pub default_member_paths: Vec<String>,
    pub available_features: Vec<String>,
    pub default_features: Vec<String>,
    pub target: Option<CargoTarget>,
}

fn document(path: &str, scan: &mut Scan) -> anyhow::Result<Value> {
    // TOML has a bounded nesting limit. The shared reader owns file/byte limits.
    let bytes = scan.read(path)?;
    Ok(Value::Table(std::str::from_utf8(&bytes)?.parse::<toml::Table>()?))
}

fn strings(value: Option<&Value>, name: &str) -> anyhow::Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let list = value.as_array().ok_or_else(|| anyhow::anyhow!("{name} must be an array of strings"))?;
    list.iter()
        .map(|value| value.as_str().map(str::to_owned).ok_or_else(|| anyhow::anyhow!("{name} must contain strings")))
        .collect()
}

pub(crate) fn read_tracked(
    path: &str,
    stop: &str,
    workspace: Option<&Workspace>,
    scan: &mut Scan,
) -> (Vec<Command>, Option<anyhow::Error>) {
    match read(path, stop, workspace, scan) {
        Ok(commands) => (commands, None),
        Err(error) => (Vec::new(), Some(error)),
    }
}

fn read(path: &str, stop: &str, workspace: Option<&Workspace>, scan: &mut Scan) -> anyhow::Result<Vec<Command>> {
    let doc = document(path, scan)?;
    let dir = filepath::dir(path);
    let enclosing = if workspace.is_none() { enclosing(&dir, stop, scan) } else { None };
    let workspace = workspace.or(enclosing.as_ref());
    if let Some(workspace) = workspace {
        scan.read(&workspace.rules.manifest)?;
    }
    let mut cursor = PathBuf::from(&dir);
    loop {
        for name in [
            "Cargo.toml",
            "Cargo.lock",
            ".cargo/config",
            ".cargo/config.toml",
            "rust-toolchain",
            "rust-toolchain.toml",
            "mise.toml",
            ".mise.toml",
            ".tool-versions",
        ] {
            scan.watch(&cursor.join(name).to_string_lossy());
        }
        if cursor == Path::new(stop) || !cursor.starts_with(stop) || !cursor.pop() {
            break;
        }
    }
    let base = CargoDetails {
        manifest_directory: canonical_path(&dir),
        workspace_root: workspace.map(|ws| canonical_path(&ws.root)),
        package: None,
        scope: CargoScope::Package,
        default_member_patterns: workspace.and_then(|ws| ws.defaults.clone()),
        default_member_paths: workspace.map(|ws| ws.default_paths.clone()).unwrap_or_default(),
        available_features: Vec::new(),
        default_features: Vec::new(),
        target: None,
    };
    let mut commands = Vec::new();
    if doc.get("workspace").is_some() {
        for (scope, label, extra) in [
            (CargoScope::DefaultMembers, "default members", Vec::new()),
            (CargoScope::AllMembers, "all members", vec!["--workspace".into()]),
        ] {
            for action in ["build", "check", "test"] {
                let mut details = base.clone();
                details.scope = scope;
                let argv = [
                    vec!["cargo".into(), action.into(), "--manifest-path".into(), canonical_path(path)],
                    extra.clone(),
                ]
                .concat();
                push(
                    &mut commands,
                    format!("{action} ({label})"),
                    format!("Generated Cargo action · {label}"),
                    argv,
                    action == "test",
                    details,
                    path,
                    scan,
                );
            }
        }
    }
    let Some(package) = doc.get("package") else {
        anyhow::ensure!(doc.get("workspace").is_some(), "manifest needs a package or workspace");
        return Ok(commands);
    };
    let name =
        package.get("name").and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("package.name must be a string"))?;
    anyhow::ensure!(
        !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_'),
        "invalid Cargo package name {name:?}"
    );
    let mut details = base;
    details.package = Some(name.into());
    details.available_features = features(&doc);
    details.default_features =
        strings(doc.get("features").and_then(|features| features.get("default")), "features.default")?;
    let description = package.get("description").and_then(Value::as_str).unwrap_or_default();
    for action in ["build", "check", "test"] {
        push(
            &mut commands,
            format!("{action} {name}"),
            format!(
                "Generated Cargo {action} · package {name}{}",
                if description.is_empty() { String::new() } else { format!(" · {description}") }
            ),
            invocation(action, path, name),
            action == "test",
            details.clone(),
            path,
            scan,
        );
    }
    let targets = read_targets(&doc, path, stop, workspace, scan);
    let binaries: Vec<_> = targets.iter().filter(|target| target.kind == TargetKind::Binary).collect();
    if binaries.len() > 1 && !binaries.iter().any(|target| target.default_run) {
        scan.notice(path, "multiple Cargo binaries; select a named --bin target");
    }
    for target in targets {
        let (action, flag) = match target.kind {
            TargetKind::Binary => ("run", "--bin"),
            TargetKind::Example if target.runnable => ("run", "--example"),
            TargetKind::Example => ("build", "--example"),
            TargetKind::Test => ("test", "--test"),
            TargetKind::Benchmark => ("bench", "--bench"),
            TargetKind::Library => ("test", "--lib"),
        };
        let mut argv = invocation(action, path, name);
        argv.push(flag.into());
        if target.kind != TargetKind::Library {
            argv.push(target.name.clone());
        }
        if !target.required_features.is_empty() {
            argv.extend(["--features".into(), target.required_features.join(",")]);
        }
        let mut desc = format!("Generated Cargo target · {name}");
        if !target.required_features.is_empty() {
            desc += &format!(" · enables required features: {}", target.required_features.join(", "));
        }
        let label = if target.kind == TargetKind::Library {
            format!("test --lib {name}")
        } else {
            format!("{action} {flag} {} ({name})", target.name)
        };
        let mut details = details.clone();
        details.target = Some(target);
        push(&mut commands, label, desc, argv, matches!(action, "run" | "test" | "bench"), details, path, scan);
    }
    Ok(commands)
}

fn invocation(action: &str, path: &str, package: &str) -> Vec<String> {
    vec![
        "cargo".into(),
        action.into(),
        "--manifest-path".into(),
        canonical_path(path),
        "--package".into(),
        package.into(),
    ]
}

fn push(
    commands: &mut Vec<Command>,
    name: String,
    desc: String,
    argv: Vec<String>,
    separator: bool,
    details: CargoDetails,
    path: &str,
    scan: &mut Scan,
) {
    if !scan.expand(Source::Cargo, path) {
        return;
    }
    let variant = format!(
        "{}:{}:{}",
        details.scope.as_str(),
        argv[1],
        details.target.as_ref().map(|target| format!("{}:{}", target.kind.as_str(), target.name)).unwrap_or_default()
    );
    commands.push(Command {
        source: Source::Cargo,
        name,
        run: argv.iter().map(|arg| quote(arg)).collect::<Vec<_>>().join(" "),
        desc,
        argv,
        args_separator: separator,
        defining_file: path.into(),
        defining_line: None,
        variant,
        generated: true,
        aliases: Vec::new(),
        hook: false,
        requirements: vec!["cargo".into(), "rustc".into()],
        cargo: Some(details),
        cargo_alias: None,
    });
}

#[cfg(test)]
mod tests;
