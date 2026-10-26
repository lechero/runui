//! Effective aliases belong to invocation cwd, rather than to every manifest
//! below a workspace. Reading configuration never executes an alias.
use std::collections::BTreeMap;
use std::path::PathBuf;

use runui_core::golang::filepath;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::discovery::Scan;
use crate::{Command, Discovery, File, Source, quote};

mod config;

#[derive(Clone, Debug)]
pub struct CargoAliasContext {
    pub directory: String,
    pub checkout: String,
    pub environment: CargoAliasEnvironment,
    pub include_global: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CargoAliasEnvironment {
    pub cargo_home: Option<PathBuf>,
    pub values: BTreeMap<String, String>,
    pub invalid_environment: Vec<String>,
}

impl CargoAliasEnvironment {
    pub fn current() -> Self {
        let mut environment = BTreeMap::new();
        let mut invalid_environment = Vec::new();
        for (key, value) in std::env::vars_os() {
            let Some(key) = key.to_str().filter(|key| key.starts_with("CARGO_ALIAS_")) else {
                continue;
            };
            match value.into_string() {
                Ok(value) => {
                    environment.insert(key.into(), value);
                }
                Err(_) => invalid_environment.push(key.into()),
            }
        }
        Self {
            cargo_home: std::env::var_os("CARGO_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo"))),
            values: environment,
            invalid_environment,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CargoAliasValue {
    String(String),
    Array(Vec<String>),
}

impl CargoAliasValue {
    fn words(&self) -> Vec<String> {
        match self {
            Self::String(value) => value.split_whitespace().take(1001).map(str::to_owned).collect(),
            Self::Array(values) => values.iter().take(1001).cloned().collect(),
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::String(value) => value.len(),
            Self::Array(values) => values.iter().map(String::len).sum(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoAliasOrigin {
    pub file: Option<String>,
    pub environment: Option<String>,
    pub project: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoAlias {
    pub name: String,
    pub value: CargoAliasValue,
    pub origins: Vec<CargoAliasOrigin>,
    pub global: bool,
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub disabled_reason: Option<String>,
}

pub fn discover_aliases(context: &CargoAliasContext) -> Discovery {
    let mut scan = Scan::cargo_aliases(&context.directory);
    scan.begin_definition();
    let configs = config::paths(context, &mut scan);
    // No project configuration means ordinary search need not read personal
    // Cargo settings. Global-only aliases require the explicit switch.
    if !context.include_global && !configs.iter().any(|path| path.project) {
        return scan.finish(Vec::new());
    }
    let mut aliases = config::load(&configs, &mut scan);
    for key in &context.environment.invalid_environment {
        scan.problem(
            Some(Source::Cargo),
            &context.directory,
            format!("Cargo alias environment variable {key:?} is not UTF-8"),
        );
    }
    let mut environment_bytes = 0usize;
    let mut expansion_bytes = 0usize;
    let mut environment_limited = false;
    'environment: for (key, value) in &context.environment.values {
        environment_bytes = environment_bytes.saturating_add(key.len()).saturating_add(value.len()).saturating_add(64);
        if environment_bytes > 1024 * 1024 {
            scan.problem(Some(Source::Cargo), &context.directory, "Cargo alias environment reached the 1-MiB limit");
            environment_limited = true;
            break;
        }
        let suffix = key.trim_start_matches("CARGO_ALIAS_");
        if suffix.is_empty() || environment_key(&suffix.to_lowercase().replace('_', "-")) != *key {
            continue;
        }
        let matching: Vec<_> = aliases.keys().filter(|name| environment_key(name) == *key).cloned().collect();
        let names = if matching.is_empty() { vec![suffix.to_lowercase().replace('_', "-")] } else { matching };
        for name in names {
            expansion_bytes = expansion_bytes
                .saturating_add(key.len())
                .saturating_add(name.len())
                .saturating_add(value.len())
                .saturating_add(64);
            if expansion_bytes > 1024 * 1024 {
                scan.problem(
                    Some(Source::Cargo),
                    &context.directory,
                    "Cargo alias environment expansion reached the 1-MiB limit",
                );
                environment_limited = true;
                break 'environment;
            }
            let origin = CargoAliasOrigin { file: None, environment: Some(key.clone()), project: false };
            if let Some(alias) = aliases.get_mut(&name) {
                match &mut alias.value {
                    CargoAliasValue::String(existing) => *existing = value.clone(),
                    CargoAliasValue::Array(existing) => {
                        let words: Vec<_> = value.split_whitespace().take(1001).map(str::to_owned).collect();
                        if existing.len().saturating_add(words.len()) > 1000 {
                            alias.disabled_reason = Some("Cargo alias exceeds 1,000 effective arguments.".into());
                        } else {
                            existing.extend(words);
                        }
                    }
                }
                alias.origins.push(origin);
            } else if aliases.len() < 1000 {
                aliases.insert(
                    name.clone(),
                    CargoAlias {
                        name,
                        value: CargoAliasValue::String(value.clone()),
                        origins: vec![origin],
                        global: true,
                        diagnostics: Vec::new(),
                        disabled_reason: None,
                    },
                );
            } else {
                scan.problem(Some(Source::Cargo), &context.directory, "Cargo alias limit reached (1,000)");
            }
        }
    }
    if environment_limited {
        for alias in aliases.values_mut() {
            alias.disabled_reason =
                Some("Cargo alias environment exceeds the inspection budget; its expansion is incomplete.".into());
        }
    }
    let cycles: Vec<_> = aliases
        .keys()
        .filter(|start| {
            let mut seen = std::collections::BTreeSet::new();
            let mut name = start.as_str();
            while let Some(alias) = aliases.get(name) {
                if config::builtin(name) {
                    break;
                }
                if !seen.insert(name) {
                    return true;
                }
                name = match &alias.value {
                    CargoAliasValue::String(value) => {
                        let mut words = value.split_whitespace();
                        let name = words.next().unwrap_or_default();
                        if words.next().is_some() {
                            break;
                        }
                        name
                    }
                    CargoAliasValue::Array(values) if values.len() == 1 => &values[0],
                    _ => break,
                };
            }
            false
        })
        .cloned()
        .collect();
    for name in cycles {
        aliases.get_mut(&name).expect("cycle name came from aliases").disabled_reason =
            Some("Cargo alias recursion contains a cycle.".into());
    }
    let mut configuration = Sha256::new();
    configuration.update(scan.definition_fingerprint().as_bytes());
    // Recursive aliases can refer to a global/environment-only alias even
    // when that alias is hidden from the current search scope.
    let mut fingerprint_bytes = 0usize;
    for (key, value) in &context.environment.values {
        fingerprint_bytes = fingerprint_bytes.saturating_add(key.len()).saturating_add(value.len()).saturating_add(64);
        if fingerprint_bytes > 1024 * 1024 {
            configuration.update(b"environment limit");
            break;
        }
        configuration.update((key.len() as u64).to_be_bytes());
        configuration.update(key.as_bytes());
        configuration.update((value.len() as u64).to_be_bytes());
        configuration.update(value.as_bytes());
    }
    let config_fingerprint = configuration.finalize();
    let mut files = Vec::new();
    for (_, mut alias) in aliases {
        alias.global = !alias.origins.iter().any(|origin| origin.project);
        if alias.global && !context.include_global {
            continue;
        }
        let words = alias.value.words();
        if words.len() > 1000 {
            alias.disabled_reason = Some("Cargo alias exceeds 1,000 effective arguments.".into());
        }
        if words.is_empty() {
            alias.disabled_reason = Some("Cargo requires a subcommand; this alias is empty.".into());
        }
        if words.first().is_some_and(|word| word.starts_with('!')) {
            alias.diagnostics.push(
                "Cargo does not interpret !command as a shell alias; it will look for that Cargo subcommand.".into(),
            );
        }
        if config::builtin(&alias.name) {
            alias.disabled_reason =
                Some("Cargo ignores this alias because a built-in command has the same name.".into());
        }
        if alias.name.is_empty() || alias.name.starts_with(['-', '+']) || alias.name.chars().any(char::is_control) {
            scan.problem(
                Some(Source::Cargo),
                &context.directory,
                format!("unsupported Cargo alias name {:?}", alias.name),
            );
            continue;
        }
        let definition = alias.origins.iter().rev().find_map(|origin| origin.file.clone()).unwrap_or_default();
        let diagnostic_path = if definition.is_empty() { &context.directory } else { &definition };
        if let Some(reason) = &alias.disabled_reason {
            scan.problem(Some(Source::Cargo), diagnostic_path, format!("Cargo alias {:?}: {reason}", alias.name));
        }
        for message in &alias.diagnostics {
            scan.notice(diagnostic_path, format!("Cargo alias {:?}: {message}", alias.name));
        }
        let argv = vec!["cargo".into(), alias.name.clone()];
        let run = argv.iter().map(|word| quote(word)).collect::<Vec<_>>().join(" ");
        let mut hash = Sha256::new();
        hash.update(config_fingerprint);
        hash.update(serde_json::to_vec(&alias).expect("alias contains only serializable values"));
        let fingerprint = hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
        let description = format!(
            "{}Cargo alias · {}",
            if alias.global { "Global " } else { "Project " },
            match &alias.value {
                CargoAliasValue::String(value) => value.clone(),
                CargoAliasValue::Array(values) => format!("{:?}", values),
            }
        );
        let native_name = alias.name.clone();
        files.push(File {
            source: Source::Cargo,
            path: definition.clone(),
            dir: context.directory.clone(),
            rel: filepath::rel(&context.checkout, &context.directory).unwrap_or_default(),
            label: "Cargo aliases".into(),
            commands: vec![Command {
                source: Source::Cargo,
                name: native_name.clone(),
                run,
                desc: description,
                argv,
                args_separator: false,
                defining_file: definition,
                defining_line: None,
                variant: format!("alias:{native_name}"),
                generated: false,
                aliases: alias
                    .origins
                    .iter()
                    .filter_map(|origin| origin.environment.as_ref())
                    .map(|name| name.trim_start_matches("CARGO_ALIAS_").to_lowercase())
                    .filter(|name| name != &native_name)
                    .collect(),
                hook: false,
                requirements: vec!["cargo".into()],
                cargo: None,
                cargo_alias: Some(alias),
            }],
            err: None,
            fingerprint,
        });
    }
    scan.finish(files)
}

fn environment_key(name: &str) -> String {
    format!("CARGO_ALIAS_{}", name.to_uppercase().replace(['-', '.'], "_"))
}

#[cfg(test)]
mod tests;
