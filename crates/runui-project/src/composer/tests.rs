use super::*;
use crate::discover;
use std::fs;

fn write(root: &str, name: &str, text: &str) {
    let path = Path::new(root).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn native_definitions_remain_intact_with_canonical_aliases_and_exact_hooks() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_str().unwrap();
    write(
        dir,
        "composer.json",
        r#"{
        "scripts": {
            "check": ["@php check.php", "App\\Checks::run", "@other"],
            "other": "echo other",
            "pre-deploy": "echo custom",
            "post-install-cmd": "App\\Hooks::install",
            "bad": {"not": "a script"}
        },
        "scripts-descriptions": {"check": "All checks", "other": 3},
        "scripts-aliases": {"check": ["c", "verify"], "other": ["c", "check", "o"], "absent": ["missing"]}
    }"#,
    );
    let discovery = discover(dir, dir);
    let file = &discovery.files[0];
    assert_eq!(file.commands.len(), 4);
    let check = &file.commands[0];
    assert_eq!(check.name, "check");
    assert_eq!(check.desc, "All checks");
    assert_eq!(check.aliases, ["c", "verify"]);
    assert_eq!(
        check.argv,
        ["env", &format!("COMPOSER={}/composer.json", canonical_path(dir)), "composer", "run-script", "--", "check"]
    );
    assert!(!check.args_separator);
    assert!(!check.run.contains("check.php"));
    assert_eq!(check.requirements, ["composer", "php"]);
    assert_eq!(file.commands[1].aliases, ["o"]);
    assert!(!file.commands[2].hook);
    assert!(file.commands[3].hook);
    assert_eq!(file.commands[3].variant, "hook");
    assert_eq!(discovery.diagnostics.len(), 5);
}

#[test]
fn explicit_manifest_and_phar_keep_the_configured_cwd_and_refresh_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_str().unwrap();
    write(
        dir,
        ".runui.json",
        r#"{"version":1,"composer":{"manifest":"config/composer.dev.json","phar":"tools/composer.phar"}}"#,
    );
    write(dir, "config/composer.dev.json", r#"{"scripts":{"test":"echo correct"}}"#);
    write(dir, "composer.json", r#"{"scripts":{"wrong":"echo wrong"}}"#);
    write(dir, "tools/composer.phar", "a disposable placeholder for passive parsing");
    let discovery = discover(dir, dir);
    assert!(discovery.complete(), "{:?}", discovery.diagnostics);
    assert_eq!(discovery.files.len(), 1);
    let file = &discovery.files[0];
    assert_eq!(file.rel_path(), "config/composer.dev.json");
    assert_eq!(canonical_path(&file.dir), canonical_path(dir));
    assert_eq!(file.commands[0].requirements, ["php"]);
    assert_eq!(file.commands[0].argv[2], "php");
    assert!(file.commands[0].argv[3].ends_with("/tools/composer.phar"));
    assert!(!discovery.changed());
    write(dir, "tools/composer.phar", "changed selected tool");
    assert!(discovery.changed());
    let second = discover(dir, dir);
    assert_ne!(second.files[0].fingerprint, file.fingerprint);
    write(dir, ".mise.toml", "[tools]\nphp = '8.4'\n");
    assert!(second.changed());
}

#[test]
fn invalid_context_and_malformed_manifests_keep_other_sources() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_str().unwrap();
    write(dir, "composer.json", "{");
    write(dir, "Makefile", "test:\n\ttrue\n");
    write(dir, "packages/api/composer.json", r#"{"scripts":{"api":"true"}}"#);
    write(dir, "vendor/dep/composer.json", r#"{"scripts":{"never":"true"}}"#);
    let discovery = discover(dir, dir);
    assert_eq!(discovery.files.len(), 3);
    assert!(discovery.files.iter().any(|file| file.source == Source::Composer && file.err.is_some()));
    assert_eq!(
        discovery.files.iter().flat_map(|file| &file.commands).map(|command| command.name.as_str()).collect::<Vec<_>>(),
        ["test", "api"]
    );
    for context in [
        r#"{"version":2,"composer":{}}"#,
        r#"{"version":1,"composer":{"manifest":"../outside.json"}}"#,
        r#"{"version":1,"composer":{"phar":false}}"#,
    ] {
        write(dir, ".runui.json", context);
        let discovery = discover(dir, dir);
        assert!(!discovery.complete());
        assert!(discovery.files.iter().any(|file| file.source == Source::Make));
        assert!(discovery.diagnostics.iter().any(|problem| problem.path.ends_with(".runui.json")));
    }
}

#[test]
fn a_missing_selected_manifest_is_visible_and_a_symlink_cannot_escape_the_folder() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("project");
    fs::create_dir(&dir).unwrap();
    let dir = dir.to_str().unwrap();
    write(dir, ".runui.json", r#"{"version":1,"composer":{"manifest":"missing.json"}}"#);
    let discovery = discover(dir, dir);
    assert!(discovery.files[0].err.is_some());
    write(dir, "missing.json", r#"{"scripts":{"new":"true"}}"#);
    assert!(discovery.changed());
    std::os::unix::fs::symlink(temp.path(), Path::new(dir).join("escape")).unwrap();
    write(dir, ".runui.json", r#"{"version":1,"composer":{"manifest":"escape/outside.json"}}"#);
    assert!(discover(dir, dir).diagnostics.iter().any(|problem| problem.message.contains("stay inside")));
}

#[test]
fn duplicate_keys_follow_composer_last_value_semantics_without_reordering_rows() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_str().unwrap();
    write(dir, "composer.json", r#"{"scripts":{"first":"before","second":"two","first":"after"}}"#);
    let (commands, error) = read_composer(&filepath::join(&[dir, "composer.json"]));
    assert!(error.is_none());
    assert_eq!(
        commands.iter().map(|command| (&*command.name, &*command.desc)).collect::<Vec<_>>(),
        [("first", "after"), ("second", "two")]
    );
}
