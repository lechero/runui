//! Static target inference. The action generator and native parity verifier
//! consume one representation for runnable and library targets alike.
use super::{CargoTarget, TargetKind, Workspace, document, strings};
use crate::discovery::Scan;
use crate::{Source, canonical_path, is_file};
use runui_core::golang::filepath;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use toml::Value;

pub(super) fn features(doc: &Value) -> Vec<String> {
    let mut names: BTreeSet<String> = doc
        .get("features")
        .and_then(Value::as_table)
        .map(|features| features.keys().cloned().collect())
        .unwrap_or_default();
    let suppressed: BTreeSet<_> = doc
        .get("features")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(|features| features.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|value| value.strip_prefix("dep:"))
        .collect();
    let mut tables = vec![doc];
    if let Some(targets) = doc.get("target").and_then(Value::as_table) {
        tables.extend(targets.values());
    }
    for table in tables {
        for key in ["dependencies", "build-dependencies"] {
            if let Some(deps) = table.get(key).and_then(Value::as_table) {
                for (name, value) in deps {
                    if value.get("optional").and_then(Value::as_bool) == Some(true)
                        && !suppressed.contains(name.as_str())
                    {
                        names.insert(name.clone());
                    }
                }
            }
        }
    }
    names.into_iter().collect()
}

pub(super) fn read_targets(
    doc: &Value,
    path: &str,
    stop: &str,
    workspace: Option<&Workspace>,
    scan: &mut Scan,
) -> Vec<CargoTarget> {
    let dir = filepath::dir(path);
    let package = doc.get("package").expect("package checked by caller");
    let name = package.get("name").and_then(Value::as_str).unwrap_or_default();
    let root_doc = workspace.and_then(|ws| document(&ws.rules.manifest, scan).ok());
    let edition = package
        .get("edition")
        .and_then(Value::as_str)
        .or_else(|| {
            package
                .get("edition")
                .and_then(|edition| edition.get("workspace"))
                .and_then(Value::as_bool)
                .filter(|inherited| *inherited)
                .and_then(|_| root_doc.as_ref()?.get("workspace")?.get("package")?.get("edition")?.as_str())
        })
        .unwrap_or("2015");
    let mut found = BTreeMap::<(TargetKind, String), CargoTarget>::new();
    let spec = [
        (TargetKind::Library, "lib", "src", "autolib"),
        (TargetKind::Binary, "bin", "src/bin", "autobins"),
        (TargetKind::Example, "example", "examples", "autoexamples"),
        (TargetKind::Test, "test", "tests", "autotests"),
        (TargetKind::Benchmark, "bench", "benches", "autobenches"),
    ];
    for (kind, key, folder, auto) in spec {
        let explicit: Vec<_> = if kind == TargetKind::Library {
            doc.get(key).into_iter().collect()
        } else {
            match doc.get(key) {
                Some(Value::Array(items)) => items.iter().collect(),
                None => Vec::new(),
                _ => {
                    scan.problem(Some(Source::Cargo), path, format!("{key} must be an array of target tables"));
                    Vec::new()
                }
            }
        };
        for target in explicit {
            let target_name = target
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| (kind == TargetKind::Library).then(|| name.replace('-', "_")));
            let Some(target_name) = target_name else {
                scan.problem(Some(Source::Cargo), path, format!("{key} target needs a name"));
                continue;
            };
            let source = target
                .get("path")
                .and_then(Value::as_str)
                .map(|path| filepath::join(&[&dir, path]))
                .unwrap_or_else(|| inferred_path(&dir, kind, folder, &target_name, name, scan));
            let crate_types = match strings(target.get("crate-type"), "crate-type") {
                Ok(types) if !types.is_empty() => types,
                Ok(_) => vec![if kind == TargetKind::Library
                    && target.get("proc-macro").and_then(Value::as_bool) == Some(true)
                {
                    "proc-macro".into()
                } else if kind == TargetKind::Library {
                    "lib".into()
                } else {
                    "bin".into()
                }],
                Err(error) => {
                    scan.problem(Some(Source::Cargo), path, error.to_string());
                    continue;
                }
            };
            let required_features = match strings(target.get("required-features"), "required-features") {
                Ok(features) if kind != TargetKind::Library => features,
                Ok(_) => Vec::new(),
                Err(error) => {
                    scan.problem(Some(Source::Cargo), path, error.to_string());
                    continue;
                }
            };
            add_target(
                &mut found,
                CargoTarget {
                    kind,
                    name: target_name,
                    source_path: source,
                    crate_types,
                    required_features,
                    runnable: false,
                    default_run: false,
                },
                path,
                stop,
                scan,
            );
        }
        let enabled = package
            .get(auto)
            .and_then(Value::as_bool)
            .unwrap_or(kind == TargetKind::Library || edition != "2015" || doc.get(key).is_none());
        if !enabled {
            continue;
        }
        let mut inferred = Vec::new();
        if kind == TargetKind::Library {
            inferred.push((name.replace('-', "_"), filepath::join(&[&dir, "src/lib.rs"])));
        } else {
            if kind == TargetKind::Binary {
                inferred.push((name.into(), filepath::join(&[&dir, "src/main.rs"])));
            }
            let directory = filepath::join(&[&dir, folder]);
            scan.watch(&directory);
            if Path::new(&directory).is_dir()
                && !std::fs::symlink_metadata(&directory).is_ok_and(|m| m.file_type().is_symlink())
                && scan.enter_folder(&directory)
            {
                match std::fs::read_dir(&directory) {
                    Ok(entries) => {
                        let mut entries: Vec<_> = entries.take(1001).collect();
                        if entries.len() > 1000 {
                            scan.problem(
                                Some(Source::Cargo),
                                &directory,
                                "target directory exceeds the 1,000-entry limit",
                            );
                            entries.truncate(1000);
                        }
                        entries.sort_by_key(|entry| entry.as_ref().map(|entry| entry.path()).unwrap_or_default());
                        for entry in entries {
                            let entry = match entry {
                                Ok(entry) => entry,
                                Err(error) => {
                                    scan.problem(Some(Source::Cargo), &directory, error.to_string());
                                    continue;
                                }
                            };
                            let source = entry.path();
                            let Some(file_name) = source.file_name().and_then(|name| name.to_str()) else {
                                scan.problem(Some(Source::Cargo), &directory, "target filename is not UTF-8");
                                continue;
                            };
                            if file_name.starts_with('.') {
                                continue;
                            }
                            if source.extension().is_some_and(|ext| ext == "rs") && source.is_file() {
                                inferred.push((
                                    source.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
                                    source.to_string_lossy().into_owned(),
                                ));
                            } else if source.is_dir()
                                && !entry.file_type().is_ok_and(|kind| kind.is_symlink())
                                && scan.enter_folder(&source.to_string_lossy())
                            {
                                let main = source.join("main.rs");
                                scan.watch(&main.to_string_lossy());
                                if main.is_file() {
                                    inferred.push((file_name.into(), main.to_string_lossy().into_owned()));
                                }
                            }
                        }
                    }
                    Err(error) => scan.problem(Some(Source::Cargo), &directory, error.to_string()),
                }
            }
        }
        let explicit: BTreeSet<_> = found.keys().cloned().collect();
        let mut inferred_names = BTreeSet::new();
        for (name, source) in inferred {
            scan.watch(&source);
            if !is_file(&source) {
                continue;
            }
            if explicit.contains(&(kind, name.clone()))
                || found
                    .values()
                    .any(|target| target.kind == kind && canonical_path(&target.source_path) == canonical_path(&source))
            {
                continue;
            }
            if !inferred_names.insert(name.clone()) {
                found.remove(&(kind, name.clone()));
                scan.problem(
                    Some(Source::Cargo),
                    path,
                    format!("multiple inferred {} targets named {name:?}", kind.as_str()),
                );
                continue;
            }
            add_target(
                &mut found,
                CargoTarget {
                    kind,
                    name,
                    source_path: source,
                    crate_types: vec![if kind == TargetKind::Library { "lib".into() } else { "bin".into() }],
                    required_features: Vec::new(),
                    runnable: false,
                    default_run: false,
                },
                path,
                stop,
                scan,
            );
        }
    }
    let default = package.get("default-run").and_then(Value::as_str);
    let binaries = found.values().filter(|target| target.kind == TargetKind::Binary).count();
    if let Some(default) = default
        && !found.contains_key(&(TargetKind::Binary, default.into()))
    {
        scan.problem(Some(Source::Cargo), path, format!("default-run refers to missing binary {default:?}"));
    }
    for target in found.values_mut() {
        target.default_run = target.kind == TargetKind::Binary
            && (default == Some(&target.name) || (default.is_none() && binaries == 1));
    }
    if found.is_empty() {
        scan.problem(Some(Source::Cargo), path, "package has no discovered targets");
    }
    found.into_values().collect()
}

fn inferred_path(dir: &str, kind: TargetKind, folder: &str, name: &str, package: &str, scan: &mut Scan) -> String {
    if kind == TargetKind::Library {
        return filepath::join(&[dir, "src/lib.rs"]);
    }
    if kind == TargetKind::Binary && name == package {
        let main = filepath::join(&[dir, "src/main.rs"]);
        scan.watch(&main);
        if is_file(&main) {
            return main;
        }
    }
    let flat = filepath::join(&[dir, folder, &format!("{name}.rs")]);
    let nested = filepath::join(&[dir, folder, name, "main.rs"]);
    scan.watch(&flat);
    scan.watch(&nested);
    if is_file(&flat) { flat } else { nested }
}

fn add_target(
    targets: &mut BTreeMap<(TargetKind, String), CargoTarget>,
    mut target: CargoTarget,
    manifest: &str,
    stop: &str,
    scan: &mut Scan,
) {
    scan.watch(&target.source_path);
    if !Path::new(&canonical_path(&target.source_path)).starts_with(canonical_path(stop)) {
        scan.problem(Some(Source::Cargo), manifest, format!("target {} is outside the checkout boundary", target.name));
        return;
    }
    if !is_file(&target.source_path) {
        scan.problem(
            Some(Source::Cargo),
            manifest,
            format!("target {} source is missing: {}", target.name, target.source_path),
        );
        return;
    }
    target.source_path = canonical_path(&target.source_path);
    target.runnable =
        target.kind == TargetKind::Binary || (target.kind == TargetKind::Example && target.crate_types == ["bin"]);
    let key = (target.kind, target.name.clone());
    if targets.insert(key, target).is_some() {
        scan.problem(Some(Source::Cargo), manifest, "duplicate target name and kind");
    }
}
