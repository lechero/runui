//! Resolve only installed tools through the supported mise launch wrapper.
//! Values returned by mise stay ephemeral and never enter the public report.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use anyhow::{anyhow, ensure};
use runui_runs::inspection::{self, End, Limits};
use serde::{Deserialize, Serialize};

use super::{Probe, Report};
use crate::{Entry, Environment, Recipe, canonical};

#[derive(Debug, Serialize)]
pub struct EnvironmentReport {
    pub wrapper: String,
    pub scope: &'static str,
    pub launch_environment_verified: bool,
    pub tools: Vec<InstalledTool>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct InstalledTool {
    pub name: String,
    pub version: String,
    pub install_path: String,
}

// The JSON environment may contain secrets from trusted global configuration.
// Do not derive Debug/Serialize, store it, or include its output in a Probe.
#[derive(Default)]
pub(super) struct NativeEnvironment {
    values: BTreeMap<String, String>,
    rust_bin: Option<String>,
}

impl NativeEnvironment {
    pub(super) fn apply(&self, command: &mut Command) {
        command.envs(&self.values);
        controls(command);
    }

    pub(super) fn path(&self) -> anyhow::Result<std::ffi::OsString> {
        self.values
            .get("PATH")
            .map(Into::into)
            .or_else(|| std::env::var_os("PATH"))
            .ok_or_else(|| anyhow!("PATH is unavailable"))
    }

    pub(super) fn resolve(&self, entry: &Entry, name: &str) -> Option<String> {
        if matches!(name, "cargo" | "rustc")
            && let Some(bin) = &self.rust_bin
        {
            let path = Path::new(bin).join(name);
            // An explicit installed Rust selection must never fall back to a
            // different PATH tool if its installation is incomplete.
            return executable(&path).then(|| path.to_string_lossy().into_owned());
        }
        let environment =
            Environment { path: std::env::split_paths(&self.path().ok()?).collect(), ..Environment::default() };
        environment.resolve(name, &entry.directory)
    }
}

fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub(super) fn binary(path: &str) -> anyhow::Result<()> {
    ensure!(executable(Path::new(path)), "tool is not an installed executable");
    let mut header = [0; 2];
    fs::File::open(path)?.read_exact(&mut header)?;
    ensure!(&header != b"#!", "script shims remain unchecked; native inspection requires an installed binary");
    ensure!(
        Path::new(&canonical(path)).file_name().is_none_or(|file| file != "mise"),
        "mise shims remain unchecked; select an installed tool"
    );
    Ok(())
}

fn controls(command: &mut Command) {
    for (key, value) in [
        ("MISE_SAFE", "1"),
        ("MISE_OFFLINE", "1"),
        ("MISE_AUTO_INSTALL", "0"),
        ("MISE_EXEC_AUTO_INSTALL", "0"),
        ("MISE_NO_HOOKS", "1"),
        ("MISE_ENV_CACHE", "0"),
        ("MISE_SELF_UPDATE_AUTO", "0"),
        ("MISE_AUTO_UPDATE", "0"),
        ("MISE_COLOR", "0"),
        ("MISE_LOG_LEVEL", "error"),
        ("RUSTUP_AUTO_INSTALL", "0"),
        ("CARGO_NET_OFFLINE", "true"),
        ("COMPOSER_DISABLE_NETWORK", "1"),
    ] {
        command.env(key, value);
    }
}

fn supported_version(text: &str) -> bool {
    let Some(version) = text.split_whitespace().next() else { return false };
    let parts = version.split('.').map(str::parse::<u32>).collect::<Result<Vec<_>, _>>();
    parts.is_ok_and(|parts| parts.len() == 3 && parts.as_slice() >= [2026, 10, 19].as_slice())
}

#[derive(Deserialize)]
struct Tool {
    version: String,
    install_path: String,
    installed: bool,
    active: bool,
}

fn tools(text: &str) -> anyhow::Result<Vec<InstalledTool>> {
    let selected: BTreeMap<String, Vec<Tool>> =
        serde_json::from_str(text).map_err(|_| anyhow!("mise returned unsupported installed-tool JSON"))?;
    ensure!(
        selected.len() <= 128 && selected.values().map(Vec::len).sum::<usize>() <= 256,
        "mise selected too many tools"
    );
    let mut result = Vec::new();
    for (name, versions) in selected {
        for tool in versions {
            ensure!(
                name.len() <= 256 && tool.version.len() <= 256 && tool.install_path.len() <= 4096,
                "mise tool metadata exceeds inspection limits"
            );
            ensure!(
                tool.installed,
                "mise selected {name} {} is not installed; install it deliberately before inspecting",
                tool.version
            );
            if !tool.active {
                continue;
            }
            ensure!(
                Path::new(&tool.install_path).is_absolute() && Path::new(&tool.install_path).is_dir(),
                "mise returned an unavailable installation directory"
            );
            result.push(InstalledTool { name: name.clone(), version: tool.version, install_path: tool.install_path });
        }
    }
    Ok(result)
}

fn values(text: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let values: BTreeMap<String, String> = serde_json::from_str(text)
        .map_err(|_| anyhow!("mise returned unsupported environment JSON; values were withheld"))?;
    ensure!(values.len() <= 1024, "mise environment exceeds 1,024 variables");
    ensure!(
        values.iter().all(|(name, value)| !name.is_empty() && !name.contains(['=', '\0']) && !value.contains('\0')),
        "mise returned an invalid environment variable; values were withheld"
    );
    ensure!(values.contains_key("PATH"), "mise environment has no PATH");
    Ok(values)
}

fn validated<T>(result: anyhow::Result<T>, report: &mut Report) -> anyhow::Result<T> {
    if let Err(error) = &result
        && let Some(probe) = report.checked.last_mut()
    {
        probe.supported = false;
        probe.error = Some(error.to_string());
    }
    result
}

// Keep process exit/timing evidence, but withhold both streams even on failure:
// JSON and configuration error excerpts may include private environment values.
fn probe(mise: &str, entry: &Entry, args: &[&str], report: &mut Report, cancel: &AtomicBool) -> anyhow::Result<String> {
    let argv: Vec<String> = std::iter::once(mise).chain(args.iter().copied()).map(str::to_string).collect();
    let mut command = Command::new(mise);
    command.args(args).current_dir(if args == ["--version"] { "/" } else { &entry.directory });
    controls(&mut command);
    let result = inspection::run(&mut command, Limits { bytes: 256 * 1024, ..Limits::default() }, cancel);
    let (output, text, supported) = match result {
        Ok(mut output) => {
            let supported = output.end == End::Exited && output.code == Some(0);
            let text = std::mem::take(&mut output.stdout);
            output.stderr.clear();
            (Some(output), text, supported)
        }
        Err(_) => (None, String::new(), false),
    };
    report.checked.push(Probe {
        tool: format!("mise {}", args[0]),
        argv,
        output,
        error: (!supported).then(|| {
            "mise inspection failed, was cancelled, or exceeded its limits; private output was withheld".into()
        }),
        supported,
    });
    ensure!(supported, "mise inspection is unavailable; static discovery was kept and private output was withheld");
    Ok(text)
}

pub(super) fn resolve(entry: &Entry, report: &mut Report, cancel: &AtomicBool) -> anyhow::Result<NativeEnvironment> {
    let Recipe::Tool { shell, .. } = &entry.recipe else { return Err(anyhow!("no tool recipe")) };
    ensure!(
        shell.len() == 5
            && shell[1..] == ["exec", "--", "/bin/sh", "-c"]
            && Path::new(&shell[0]).file_name().is_some_and(|name| name == "mise"),
        "Project shell wrapper remains unverified; native inspection supports mise exec -- /bin/sh -c only"
    );
    let mise = &shell[0];
    ensure!(Path::new(mise).is_absolute(), "mise wrapper path must be absolute; refresh discovery");
    // The wrapper itself is a tool we deliberately invoke, not a selected shim.
    ensure!(executable(Path::new(mise)), "mise is not installed");
    let mut header = [0; 2];
    fs::File::open(mise)?.read_exact(&mut header)?;
    ensure!(&header != b"#!", "mise is a script shim; native inspection requires the installed mise binary");
    let version = probe(mise, entry, &["--version"], report, cancel)?;
    validated(
        if supported_version(&version) {
            Ok(())
        } else {
            Err(anyhow!(
                "Installed-tool inspection requires mise 2026.10.19 or newer with safe/offline mode; static discovery was kept"
            ))
        },
        report,
    )?;
    let selected = probe(mise, entry, &["ls", "--current", "--json"], report, cancel)?;
    let tools = validated(tools(&selected), report)?;
    let selected = probe(mise, entry, &["env", "--json"], report, cancel)?;
    let values = validated(values(&selected), report)?;
    let rust_bin = tools
        .iter()
        .find(|tool| tool.name == "rust" || tool.name == "core:rust")
        .map(|tool| Path::new(&tool.install_path).join("bin").to_string_lossy().into_owned());
    report.environment = Some(EnvironmentReport {
        wrapper: mise.clone(), scope: "installed_tools", launch_environment_verified: false, tools,
        limitations: vec!["Safe, offline mise tool selection only. Project environment values, hooks, shell aliases, dynamic templates and compiler wrappers are not evaluated; the full launch environment remains unchecked.".into(),
            "Inspection never installs missing tools. Normal launches still use the original project wrapper.".into()],
    });
    Ok(NativeEnvironment { values, rust_bin })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(root: &str, shell: &[String]) -> Entry {
        fs::write(Path::new(root).join("composer.json"), r#"{"scripts":{"check":"@php never.php"}}"#).unwrap();
        let mut context = crate::Context::new(root, root);
        context.environment = Environment::default().with_shell(shell);
        crate::Snapshot::build(&context, &runui_project::discover(root, root), &[], &[], &crate::usage::Data::default())
            .entries()
            .iter()
            .find(|entry| entry.source == "composer")
            .unwrap()
            .clone()
    }
    #[test]
    fn safe_mode_version_gate_and_json_reject_ambiguous_or_missing_tools() {
        for value in ["2026.10.19 macos-arm64", "2026.11.1 linux-x64", "2027.1.0"] {
            assert!(supported_version(value));
        }
        for value in ["2026.10.18", "2026.2.99", "2026.10.19-dev", "unknown", "2026.10.19.1"] {
            assert!(!supported_version(value));
        }
        assert!(
            tools(r#"{"rust":[{"version":"1.99.0","install_path":"/tmp","installed":false,"active":true}]}"#)
                .unwrap_err()
                .to_string()
                .contains("not installed")
        );
        // Mise marks a missing current request inactive; it still must prevent
        // an unrelated system tool from masquerading as that selected version.
        assert!(
            tools(r#"{"rust":[{"version":"99.99.99","install_path":"/tmp","installed":false,"active":false}]}"#)
                .is_err()
        );
        assert!(tools(r#"{"rust":[{"installed":true}]}"#).is_err());
        assert!(values(r#"{"PATH":"/bin","TOKEN":"secret"}"#).is_ok());
        for text in [r#"{"TOKEN":"secret"}"#, r#"{"PATH":null}"#, r#"{"PATH":"/bin","bad=name":"value"}"#] {
            assert!(values(text).is_err());
        }
        assert!(!values("private value excerpt").unwrap_err().to_string().contains("private value excerpt"));
    }

    #[test]
    fn provider_controls_override_resolved_values_and_script_shims_are_rejected() {
        let mut environment = NativeEnvironment::default();
        environment.values.insert("MISE_AUTO_INSTALL".into(), "1".into());
        environment.values.insert("MISE_SAFE".into(), "0".into());
        environment.values.insert("RUSTUP_AUTO_INSTALL".into(), "1".into());
        let mut command = Command::new("/bin/true");
        environment.apply(&mut command);
        let actual: BTreeMap<_, _> = command.get_envs().collect();
        for (name, value) in [("MISE_AUTO_INSTALL", "0"), ("MISE_SAFE", "1"), ("RUSTUP_AUTO_INSTALL", "0")] {
            assert_eq!(actual.get(std::ffi::OsStr::new(name)).unwrap().unwrap(), std::ffi::OsStr::new(value));
        }
        assert!(binary("/bin/sh").is_ok());
        let temp = tempfile::tempdir().unwrap();
        let shim = temp.path().join("php");
        fs::write(&shim, "#!/bin/sh\ntouch must-not-run\n").unwrap();
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(binary(&shim.to_string_lossy()).unwrap_err().to_string().contains("script shims"));
    }

    #[test]
    fn unknown_wrappers_and_mise_script_shims_never_run_and_missing_rust_does_not_fall_back() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy();
        let wrapped = entry(&root, &["/bin/sh".into(), "-c".into(), "echo unexpected".into()]);
        let report = super::super::check(&wrapped, &AtomicBool::new(false));
        assert!(report.checked.is_empty() && !report.ready);
        assert!(report.skipped.unwrap().contains("wrapper remains unverified"));
        let mise = temp.path().join("mise");
        fs::write(&mise, "#!/bin/sh\ntouch must-not-run\n").unwrap();
        fs::set_permissions(&mise, fs::Permissions::from_mode(0o755)).unwrap();
        let wrapped = entry(
            &root,
            &[mise.to_string_lossy().into_owned(), "exec".into(), "--".into(), "/bin/sh".into(), "-c".into()],
        );
        let report = super::super::check(&wrapped, &AtomicBool::new(false));
        assert!(report.checked.is_empty() && !report.ready);
        assert!(report.skipped.unwrap().contains("script shim"));
        assert!(!temp.path().join("must-not-run").exists());
        let environment = NativeEnvironment {
            rust_bin: Some(temp.path().join("missing").to_string_lossy().into_owned()),
            ..NativeEnvironment::default()
        };
        assert!(environment.resolve(&wrapped, "cargo").is_none());
    }
}
