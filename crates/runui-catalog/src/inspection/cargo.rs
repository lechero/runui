//! Cargo's versioned metadata is an explicit, bounded check of the selected
//! static definition. Its package IDs are opaque map keys, never paths.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, anyhow, ensure};
use runui_project::{CargoScope, CargoTarget, TargetKind};
use runui_runs::inspection::{self, End, Limits};
use serde::{Deserialize, Serialize};

use super::{Probe, Report, environment::NativeEnvironment};
use crate::{Entry, canonical};

#[derive(Debug, Serialize)]
pub struct CargoMetadata {
    pub workspace_root: String,
    pub members: Vec<String>,
    pub default_members: Vec<String>,
    pub package: Option<String>,
    pub targets: Vec<CargoTarget>,
    pub available_features: Vec<String>,
    pub default_features: Vec<String>,
    pub diagnostics: Vec<String>,
    pub definition_matches: bool,
}

#[derive(Deserialize)]
struct Metadata {
    version: u32,
    workspace_root: String,
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    workspace_default_members: Vec<String>,
}
#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: String,
    targets: Vec<Target>,
    features: BTreeMap<String, Vec<String>>,
    default_run: Option<String>,
}
#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
    crate_types: Vec<String>,
    src_path: String,
    #[serde(default, rename = "required-features")]
    required_features: Vec<String>,
}

fn metadata(text: &str, entry: &Entry) -> anyhow::Result<CargoMetadata> {
    let metadata: Metadata = serde_json::from_str(text).context("invalid Cargo metadata JSON")?;
    ensure!(metadata.version == 1, "unsupported Cargo metadata version {}", metadata.version);
    ensure!(
        metadata.packages.len() <= 1000
            && metadata.workspace_members.len() <= 1000
            && metadata.workspace_default_members.len() <= 1000,
        "Cargo metadata exceeds 1,000 members"
    );
    ensure!(
        metadata.workspace_members.iter().collect::<BTreeSet<_>>().len() == metadata.workspace_members.len()
            && metadata.workspace_default_members.iter().collect::<BTreeSet<_>>().len()
                == metadata.workspace_default_members.len(),
        "Cargo metadata repeats a workspace member ID"
    );
    let mut packages = BTreeMap::new();
    for package in metadata.packages {
        ensure!(package.targets.len() <= 1000, "Cargo metadata exceeds 1,000 targets per package");
        ensure!(packages.insert(package.id.clone(), package).is_none(), "Cargo metadata repeats a package ID");
    }
    let paths = |ids: &[String]| -> anyhow::Result<Vec<String>> {
        ids.iter()
            .map(|id| {
                let package =
                    packages.get(id).ok_or_else(|| anyhow!("Cargo metadata refers to an unknown package ID"))?;
                ensure!(
                    Path::new(&package.manifest_path).is_absolute(),
                    "Cargo metadata manifest path is not absolute"
                );
                Ok(canonical(
                    Path::new(&package.manifest_path).parent().unwrap_or(Path::new("/")).to_string_lossy().as_ref(),
                ))
            })
            .collect()
    };
    let members = paths(&metadata.workspace_members)?;
    let defaults = paths(&metadata.workspace_default_members)?;
    ensure!(
        defaults.iter().all(|default| members.contains(default)),
        "Cargo metadata defaults are not workspace members"
    );
    ensure!(Path::new(&metadata.workspace_root).is_absolute(), "Cargo metadata workspace root is not absolute");
    let root = canonical(&metadata.workspace_root);
    ensure!(Path::new(&root).starts_with(&entry.checkout), "Cargo metadata workspace is outside this checkout");
    let mut diagnostics = Vec::new();
    if members.iter().any(|member| !Path::new(member).starts_with(&entry.checkout)) {
        diagnostics.push(
            "Native workspace includes members outside this checkout; static discovery keeps its boundary.".into(),
        );
    }
    let selected = packages
        .iter()
        .find(|(id, package)| {
            metadata.workspace_members.contains(id)
                && entry.entry_file.as_ref().is_some_and(|path| canonical(&package.manifest_path) == *path)
        })
        .map(|(_, package)| package);
    let mut targets = Vec::new();
    let mut available_features = Vec::new();
    let mut default_features = Vec::new();
    if let Some(package) = selected {
        let binaries = package.targets.iter().filter(|target| target.kind == ["bin"]).count();
        for target in &package.targets {
            let kind = match target.kind.as_slice() {
                [kind] if kind == "custom-build" => continue,
                [kind] if kind == "bin" => TargetKind::Binary,
                [kind] if kind == "example" => TargetKind::Example,
                [kind] if kind == "test" => TargetKind::Test,
                [kind] if kind == "bench" => TargetKind::Benchmark,
                kinds
                    if !kinds.is_empty()
                        && kinds.iter().all(|kind| {
                            ["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"].contains(&kind.as_str())
                        }) =>
                {
                    TargetKind::Library
                }
                _ => {
                    diagnostics.push(format!(
                        "Native target {:?} has an unsupported kind; static discovery was kept.",
                        target.name
                    ));
                    continue;
                }
            };
            ensure!(Path::new(&target.src_path).is_absolute(), "Cargo metadata target path is not absolute");
            let source_path = canonical(&target.src_path);
            if !Path::new(&source_path).starts_with(&entry.checkout) {
                diagnostics
                    .push(format!("Native target {:?} is outside this checkout; it was not added.", target.name));
                continue;
            }
            targets.push(CargoTarget {
                kind,
                name: target.name.clone(),
                source_path,
                crate_types: target.crate_types.clone(),
                required_features: target.required_features.clone(),
                runnable: kind == TargetKind::Binary || (kind == TargetKind::Example && target.crate_types == ["bin"]),
                default_run: kind == TargetKind::Binary
                    && (package.default_run.as_deref() == Some(&target.name)
                        || (package.default_run.is_none() && binaries == 1)),
            });
        }
        available_features = package.features.keys().cloned().collect();
        default_features = package.features.get("default").cloned().unwrap_or_default();
    }
    let cargo = entry.cargo.as_ref().ok_or_else(|| anyhow!("selected entry has no Cargo definition"))?;
    let mut matches = match cargo.scope {
        CargoScope::Package => selected.is_some_and(|package| cargo.package.as_deref() == Some(&package.name)),
        CargoScope::DefaultMembers | CargoScope::AllMembers => cargo.workspace_root.as_deref() == Some(&root),
    };
    if let Some(target) = &cargo.target {
        matches &= targets.contains(target);
    }
    if cargo.scope == CargoScope::DefaultMembers {
        let expected: BTreeSet<_> = cargo
            .default_member_paths
            .iter()
            .map(|path| canonical(&Path::new(&root).join(path).to_string_lossy()))
            .collect();
        matches &= expected == defaults.iter().cloned().collect();
    }
    if !matches {
        diagnostics.push("Native metadata differs from this static definition. Refresh and inspect before running; static commands were kept.".into());
    }
    Ok(CargoMetadata {
        workspace_root: root,
        members,
        default_members: defaults,
        package: selected.map(|package| package.name.clone()),
        targets,
        available_features,
        default_features,
        diagnostics,
        definition_matches: matches,
    })
}

/// Retain the proxy spelling: canonicalizing a rustup symlink and executing it
/// as `rustup` changes argv[0] and selects the rustup CLI instead of Cargo.
fn tool(entry: &Entry, name: &str, environment: &NativeEnvironment) -> anyhow::Result<String> {
    let expected = entry
        .requirements
        .iter()
        .find(|requirement| requirement.program == name)
        .and_then(|requirement| requirement.resolved_path.as_deref())
        .ok_or_else(|| anyhow!("{name} is not available on PATH"))?;
    let expected = canonical(expected);
    let path = environment.path()?;
    // A mise Rust installation can use package-manager proxy directories on
    // PATH; inspect its actual installed tool instead of executing that proxy.
    let selected = environment.resolve(entry, name).into_iter().map(std::path::PathBuf::from);
    for candidate in
        selected.chain(std::env::split_paths(&path).map(|dir| Path::new(&entry.directory).join(dir).join(name)))
    {
        if canonical(&candidate.to_string_lossy()) != expected || !candidate.is_file() {
            continue;
        }
        let mut header = [0; 2];
        fs::File::open(&candidate)?.read_exact(&mut header)?;
        ensure!(
            &header != b"#!",
            "{name} is a script shim; native inspection requires an installed binary or rustup proxy"
        );
        ensure!(
            Path::new(&expected).file_name().is_none_or(|file| file != "mise"),
            "mise shims need project-wrapper verification"
        );
        return Ok(candidate.to_string_lossy().into_owned());
    }
    Err(anyhow!("{name} changed on PATH; refresh discovery before native inspection"))
}

fn probe(
    entry: &Entry,
    name: &str,
    argv: Vec<String>,
    cancel: &AtomicBool,
    bytes: usize,
    environment: &NativeEnvironment,
) -> Probe {
    let mut command = Command::new(&argv[0]);
    environment.apply(&mut command);
    command
        .args(&argv[1..])
        .current_dir(&entry.directory)
        .env("RUSTUP_AUTO_INSTALL", "0")
        .env("CARGO_NET_OFFLINE", "true")
        .env("MISE_AUTO_INSTALL", "0")
        .env("MISE_EXEC_AUTO_INSTALL", "0")
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "");
    let (output, error) = match inspection::run(&mut command, Limits { bytes, ..Limits::default() }, cancel) {
        Ok(output) => (Some(output), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let supported = output.as_ref().is_some_and(|output| output.end == End::Exited && output.code == Some(0));
    Probe { tool: name.into(), argv, output, error, supported }
}

/// `rustup which` locates an existing tool without executing a proxy's
/// install-on-demand path. Resolve symlink and hardlink proxies alike.
fn installed(
    entry: &Entry,
    name: &str,
    report: &mut Report,
    cancel: &AtomicBool,
    environment: &NativeEnvironment,
) -> anyhow::Result<String> {
    let path = tool(entry, name, environment)?;
    let sibling = Path::new(&path).with_file_name("rustup");
    let proxy = canonical(&path);
    let rustup = if Path::new(&proxy).file_name().is_some_and(|name| name == "rustup") {
        Some(proxy)
    } else if fs::metadata(&path)
        .ok()
        .zip(fs::metadata(&sibling).ok())
        .is_some_and(|(tool, rustup)| tool.dev() == rustup.dev() && tool.ino() == rustup.ino())
    {
        Some(sibling.to_string_lossy().into_owned())
    } else {
        None
    };
    let Some(rustup) = rustup else {
        return Ok(path);
    };
    let checked = probe(
        entry,
        &format!("rustup which {name}"),
        vec![rustup, "which".into(), name.into()],
        cancel,
        64 * 1024,
        environment,
    );
    let found = checked
        .output
        .as_ref()
        .filter(|_| checked.supported)
        .map(|output| output.stdout.trim_end_matches('\n').to_string());
    report.checked.push(checked);
    let found = found.ok_or_else(|| {
        anyhow!("{name} is not installed for the selected rustup toolchain; static discovery was kept")
    })?;
    ensure!(
        Path::new(&found).is_absolute() && !found.contains(['\n', '\r']),
        "rustup returned an unsupported tool path"
    );
    let metadata = fs::metadata(&found).context("rustup tool is not installed")?;
    ensure!(
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
        "rustup tool is not an installed executable"
    );
    ensure!(canonical(&found) != canonical(&path), "rustup returned a tool proxy instead of an installed tool");
    let mut header = [0; 2];
    fs::File::open(&found)?.read_exact(&mut header)?;
    ensure!(&header != b"#!", "rustup selected a script wrapper; installed binary inspection is required");
    Ok(found)
}

#[cfg(test)]
pub(super) fn check(entry: &Entry, cancel: &AtomicBool) -> Report {
    check_in(
        entry,
        cancel,
        Report { checked: Vec::new(), skipped: None, ready: false, cargo: None, environment: None },
        &NativeEnvironment::default(),
    )
}

pub(super) fn check_in(
    entry: &Entry,
    cancel: &AtomicBool,
    mut report: Report,
    environment: &NativeEnvironment,
) -> Report {
    let result = (|| -> anyhow::Result<()> {
        let definition = entry.cargo.as_ref().ok_or_else(|| anyhow!("Cargo entry has no static definition"))?;
        let root = definition.workspace_root.as_deref().unwrap_or(&definition.manifest_directory);
        ensure!(
            Path::new(root).join("Cargo.lock").is_file(),
            "Native Cargo inspection requires an existing Cargo.lock; static discovery was kept. Generate the lockfile deliberately before inspecting."
        );
        let cargo = installed(entry, "cargo", &mut report, cancel, environment)?;
        let rustc = installed(entry, "rustc", &mut report, cancel, environment)?;
        for (name, path) in [("cargo", &cargo), ("rustc", &rustc)] {
            let mut version =
                probe(entry, name, vec![path.clone(), "--version".into()], cancel, 64 * 1024, environment);
            version.supported &=
                version.output.as_ref().is_some_and(|output| output.stdout.starts_with(&format!("{name} 1.")));
            let supported = version.supported;
            report.checked.push(version);
            ensure!(supported, "{name} version check failed; static discovery was kept");
        }
        let manifest = entry.entry_file.as_ref().ok_or_else(|| anyhow!("Cargo entry has no manifest"))?;
        let argv = vec![
            cargo,
            "metadata".into(),
            "--no-deps".into(),
            "--offline".into(),
            "--locked".into(),
            "--format-version".into(),
            "1".into(),
            "--manifest-path".into(),
            manifest.clone(),
            "--color".into(),
            "never".into(),
            "--config".into(),
            format!("build.rustc={}", serde_json::to_string(&rustc)?),
            "--config".into(),
            "build.rustc-wrapper=\"\"".into(),
            "--config".into(),
            "build.rustc-workspace-wrapper=\"\"".into(),
        ];
        let mut checked = probe(entry, "cargo metadata", argv, cancel, 1024 * 1024, environment);
        let parsed = if checked.supported {
            metadata(&checked.output.as_ref().expect("successful probe has output").stdout, entry)
        } else {
            Err(anyhow!(
                "Offline locked metadata is unavailable; static discovery was kept. Check the lockfile, local cache, and installed toolchain."
            ))
        };
        match parsed {
            Ok(metadata) => {
                checked.supported = metadata.definition_matches;
                report.ready = metadata.definition_matches;
                report.cargo = Some(metadata);
            }
            Err(error) => {
                checked.supported = false;
                checked.error = Some(error.to_string());
            }
        }
        report.checked.push(checked);
        Ok(())
    })();
    if let Err(error) = result {
        report.skipped = Some(error.to_string());
    }
    report
}

#[cfg(test)]
mod tests;
