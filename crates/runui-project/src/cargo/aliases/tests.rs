use super::*;
use crate::canonical_path;
use std::fs;
use std::path::Path;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn context(root: &Path) -> CargoAliasContext {
    let project = root.join("checkout");
    fs::create_dir_all(&project).unwrap();
    CargoAliasContext {
        directory: canonical_path(&project.to_string_lossy()),
        checkout: canonical_path(&project.to_string_lossy()),
        environment: super::CargoAliasEnvironment {
            cargo_home: Some(root.join("cargo-home")),
            values: BTreeMap::new(),
            invalid_environment: Vec::new(),
        },
        include_global: false,
    }
}
fn aliases(discovery: &Discovery) -> BTreeMap<String, CargoAlias> {
    discovery
        .files
        .iter()
        .flat_map(|file| &file.commands)
        .map(|command| (command.name.clone(), command.cargo_alias.clone().unwrap()))
        .collect()
}

#[test]
fn environment_collisions_share_an_expansion_budget() {
    let temp = tempfile::tempdir().unwrap();
    let mut ctx = context(temp.path());
    write(&Path::new(&ctx.directory).join(".cargo/config.toml"), "[alias]\nsame-name='version'\nsame_name='version'\n");
    ctx.environment.values.insert("CARGO_ALIAS_SAME_NAME".into(), "x".repeat(600_000));
    let discovery = discover_aliases(&ctx);
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("environment expansion")));
    assert!(aliases(&discovery).values().all(|alias| alias.disabled_reason.is_some()));
}

#[test]
fn config_symlinks_keep_their_invocation_side_include_directory() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(temp.path());
    let root = Path::new(&ctx.directory);
    let shared = temp.path().join("shared");
    write(&shared.join("base.toml"), "include=['included.toml']\n");
    write(&shared.join("included.toml"), "[alias]\nprobe='wrong side'\n");
    write(&root.join(".cargo/included.toml"), "[alias]\nprobe='version --verbose'\n");
    std::os::unix::fs::symlink(shared.join("base.toml"), root.join(".cargo/config.toml")).unwrap();
    let found = aliases(&discover_aliases(&ctx));
    assert!(!found["probe"].global);
    assert_eq!(found["probe"].value, CargoAliasValue::String("version --verbose".into()));
}

#[test]
fn the_closest_config_is_preserved_when_the_file_budget_truncates_ancestors() {
    let temp = tempfile::tempdir().unwrap();
    let mut ctx = context(temp.path());
    let mut folder = PathBuf::from(&ctx.directory);
    for level in 0..64 {
        write(&folder.join(".cargo/config.toml"), &format!("[alias]\nparent{level}='version'\n"));
        if level < 63 {
            folder.push("nested");
        }
    }
    write(&folder.join(".cargo/config.toml"), "[alias]\nnearest='version'\n");
    write(&temp.path().join("cargo-home/config.toml"), "[alias]\nglobal='version'\n");
    ctx.directory = canonical_path(&folder.to_string_lossy());
    let discovery = discover_aliases(&ctx);
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("64-file limit")));
    assert!(aliases(&discovery).contains_key("nearest"));
}

#[test]
fn cargo_home_inside_a_checkout_is_still_global_unless_it_is_also_a_local_config() {
    let temp = tempfile::tempdir().unwrap();
    let mut ctx = context(temp.path());
    let root = Path::new(&ctx.directory);
    ctx.environment.cargo_home = Some(root.join("cache/cargo-home"));
    write(&root.join("cache/cargo-home/config.toml"), "[alias]\nhome-only='version'\n");
    assert!(aliases(&discover_aliases(&ctx)).is_empty());
    ctx.include_global = true;
    assert!(aliases(&discover_aliases(&ctx))["home-only"].global);
    write(&root.join(".cargo/config.toml"), "[alias]\nlocal='version'\n");
    ctx.environment.cargo_home = Some(root.join(".cargo"));
    ctx.include_global = false;
    let found = aliases(&discover_aliases(&ctx));
    assert!(!found["local"].global);
    assert_eq!(found["local"].origins.len(), 1);
}

#[test]
fn closer_strings_override_arrays_append_and_global_only_aliases_require_opt_in() {
    let temp = tempfile::tempdir().unwrap();
    let mut ctx = context(temp.path());
    write(
        &temp.path().join("cargo-home/config.toml"),
        "[alias]\njoined=['version']\nchosen='check'\nglobal='version'\n",
    );
    write(&Path::new(&ctx.directory).join(".cargo/config.toml"), "[alias]\njoined=['--verbose']\nchosen='version'\n");
    let discovery = discover_aliases(&ctx);
    assert!(discovery.complete(), "{:?}", discovery.diagnostics);
    let found = aliases(&discovery);
    assert_eq!(found.len(), 2);
    assert!(!found.contains_key("global"));
    assert_eq!(found["joined"].value, CargoAliasValue::Array(vec!["version".into(), "--verbose".into()]));
    assert_eq!(found["joined"].origins.len(), 2);
    assert!(!found["joined"].global);
    assert_eq!(found["chosen"].value, CargoAliasValue::String("version".into()));
    ctx.include_global = true;
    assert!(aliases(&discover_aliases(&ctx))["global"].global);
}

#[test]
fn environment_overrides_strings_appends_to_arrays_and_env_only_names_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let mut ctx = context(temp.path());
    write(&Path::new(&ctx.directory).join(".cargo/config.toml"), "[alias]\njoined=['version']\nchosen='check'\n");
    ctx.environment.values.insert("CARGO_ALIAS_JOINED".into(), "--verbose".into());
    ctx.environment.values.insert("CARGO_ALIAS_CHOSEN".into(), "version".into());
    ctx.environment.values.insert("CARGO_ALIAS_ENV_ONLY".into(), "version".into());
    let found = aliases(&discover_aliases(&ctx));
    assert_eq!(found["joined"].value, CargoAliasValue::Array(vec!["version".into(), "--verbose".into()]));
    assert_eq!(found["chosen"].value, CargoAliasValue::String("version".into()));
    assert!(!found.contains_key("env-only"));
    ctx.include_global = true;
    assert!(aliases(&discover_aliases(&ctx)).contains_key("env-only"));
}

#[test]
fn legacy_precedence_missing_includes_cycles_and_config_edits_refresh_without_execution() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(temp.path());
    let root = Path::new(&ctx.directory);
    write(&root.join(".cargo/config"), "include=[{path='optional.toml',optional=true}]\n[alias]\nlocal='version'\n");
    write(&root.join(".cargo/config.toml"), "[alias]\nignored='must-not-run'\n");
    let before = discover_aliases(&ctx);
    assert_eq!(aliases(&before).len(), 1);
    write(&root.join(".cargo/config.toml"), "[alias]\nignored='changed'\n");
    assert!(before.changed());
    assert_eq!(before.files[0].fingerprint, discover_aliases(&ctx).files[0].fingerprint);
    write(&root.join(".cargo/optional.toml"), "include=['nested.toml']\n[alias]\nincluded=['version']\n");
    write(&root.join(".cargo/nested.toml"), "include=['optional.toml']\n");
    let changed = discover_aliases(&ctx);
    assert!(!changed.complete());
    assert!(changed.diagnostics.iter().any(|problem| problem.message.contains("cycle")));
    assert!(aliases(&changed).contains_key("included"));
    assert_ne!(before.files[0].fingerprint, changed.files[0].fingerprint);
}

#[test]
fn mixed_types_oversized_sources_and_many_effective_arguments_are_diagnosed() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(temp.path());
    let root = Path::new(&ctx.directory);
    write(&temp.path().join("cargo-home/config.toml"), "[alias]\nmixed=['version']\n");
    write(&root.join(".cargo/config.toml"), "[alias]\nmixed='version'\ngood='version'\n");
    let discovery = discover_aliases(&ctx);
    assert!(!discovery.complete());
    assert!(!aliases(&discovery).contains_key("mixed"));
    assert!(aliases(&discovery).contains_key("good"));
    write(&root.join(".cargo/config.toml"), &"#".repeat(64 * 1024 + 1));
    assert!(discover_aliases(&ctx).diagnostics.iter().any(|problem| problem.message.contains("64-KiB")));
    write(&root.join(".cargo/config.toml"), "[alias]\ngood='version'\n");
    let mut ctx = ctx;
    ctx.environment.values.insert("CARGO_ALIAS_GOOD".into(), "arg ".repeat(1001));
    assert!(aliases(&discover_aliases(&ctx))["good"].disabled_reason.as_ref().unwrap().contains("1,000"));
}

#[test]
fn recursive_cycles_are_disabled_while_shell_style_text_stays_with_cargo() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(temp.path());
    write(
        &Path::new(&ctx.directory).join(".cargo/config.toml"),
        "[alias]\none='two'\ntwo=['one']\nshell='!echo should-not-run'\nvalid='version'\n",
    );
    let discovery = discover_aliases(&ctx);
    let found = aliases(&discovery);
    assert!(found["one"].disabled_reason.as_ref().unwrap().contains("cycle"));
    assert!(found["two"].disabled_reason.as_ref().unwrap().contains("cycle"));
    assert!(found["shell"].disabled_reason.is_none());
    assert!(found["shell"].diagnostics[0].contains("does not interpret"));
    assert!(found["valid"].diagnostics.is_empty());
}

#[test]
fn ignored_builtin_overrides_do_not_disable_aliases_that_reach_a_native_command() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(temp.path());
    write(
        &Path::new(&ctx.directory).join(".cargo/config.toml"),
        "[alias]\nthrough-version='version'\nversion='through-version'\n",
    );
    let found = aliases(&discover_aliases(&ctx));
    assert!(found["through-version"].disabled_reason.is_none());
    assert!(found["version"].disabled_reason.as_ref().unwrap().contains("built-in"));
}

#[test]
fn malformed_configuration_reports_location_without_copying_source_values() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(temp.path());
    write(&Path::new(&ctx.directory).join(".cargo/config.toml"), "[registry]\ntoken='PRIVATE_VALUE_NEVER_EXCERPT\n");
    let discovery = discover_aliases(&ctx);
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("line 2")));
    assert!(discovery.diagnostics.iter().all(|problem| !problem.message.contains("PRIVATE_VALUE_NEVER_EXCERPT")));
}
