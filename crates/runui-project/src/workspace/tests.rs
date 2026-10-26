use super::*;
use crate::discover;

fn write(dir: &Path, file: &str, contents: &str) {
    let path = dir.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn package(dir: &Path, folder: &str, name: &str) {
    write(dir, &format!("{folder}/package.json"), &format!(r#"{{"scripts":{{"{name}":"true"}}}}"#));
}

fn commands(dir: &Path) -> Vec<String> {
    discover(dir.to_str().unwrap(), dir.to_str().unwrap())
        .files
        .into_iter()
        .flat_map(|file| file.commands.into_iter().map(|command| command.name))
        .collect()
}

#[test]
fn pnpm_declares_deep_members_and_exclusions_without_a_lockfile() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    write(dir, "pnpm-workspace.yaml", "packages:\n  - 'apps/**'\n  - '!apps/**/excluded'\n");
    package(dir, "apps/a/b/c/web", "deep");
    package(dir, "apps/a/b/c/excluded", "excluded");
    package(dir, "unlisted", "incidental");
    let discovery = discover(dir.to_str().unwrap(), dir.to_str().unwrap());
    assert_eq!(commands(dir), ["deep"]);
    assert_eq!(discovery.files[0].commands[0].run, "pnpm run deep");
    assert!(discovery.complete(), "{:?}", discovery.diagnostics);
    assert_eq!(
        crate::root(&dir.join("apps/a/b/c/web/src").to_string_lossy(), "/"),
        dir.join("apps/a/b/c/web").to_string_lossy()
    );
    assert_eq!(
        workspace_root(&dir.join("apps/a/b/c/web").to_string_lossy(), &dir.to_string_lossy()),
        dir.to_string_lossy()
    );
}

#[test]
fn npm_yarn_array_and_classic_object_membership_support_braces() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    for workspaces in [
        r#"["./packages/{web,api}/*", "!packages/web/old"]"#,
        r#"{"packages":["packages/{web,api}/*", "!packages/web/old"],"nohoist":["**"]}"#,
    ] {
        write(dir, "package.json", &format!(r#"{{"packageManager":"yarn@4","workspaces":{workspaces}}}"#));
        package(dir, "packages/web/new", "web");
        package(dir, "packages/api/new", "api");
        package(dir, "packages/web/old", "old");
        package(dir, "packages/other/new", "other");
        assert_eq!(commands(dir), ["api", "web"]);
        let discovery = discover(&dir.to_string_lossy(), &dir.to_string_lossy());
        assert!(
            discovery.files.iter().all(|file| file.commands.iter().all(|command| command.run.starts_with("yarn run")))
        );
    }
}

#[test]
fn omitted_and_empty_pnpm_members_are_root_only_and_take_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    write(dir, "package.json", r#"{"scripts":{"root":"true"},"workspaces":["packages/*"]}"#);
    package(dir, "packages/web", "web");
    for contents in ["catalog: {}\n", "packages: []\n"] {
        write(dir, "pnpm-workspace.yaml", contents);
        assert_eq!(commands(dir), ["root"]);
    }
}

#[test]
fn declared_members_override_gitignore_but_not_runuiignore() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    write(dir, "package.json", r#"{"workspaces":["packages/*"]}"#);
    write(dir, ".gitignore", "packages/\nincidental/\n");
    package(dir, "packages/web", "web");
    package(dir, "packages/private", "private");
    package(dir, "incidental", "incidental");
    write(dir, ".runuiignore", "packages/private/\n");
    let discovery = discover(&dir.to_string_lossy(), &dir.to_string_lossy());
    assert_eq!(commands(dir), ["web"]);
    assert!(discovery.complete(), "{:?}", discovery.diagnostics);
    assert!(
        discovery
            .notices
            .iter()
            .any(|notice| notice.path.ends_with("packages/web") && notice.message.contains("despite"))
    );
    assert!(!discovery.changed());
    write(dir, ".runuiignore", "packages/web/\n");
    assert!(discovery.changed());
    assert_eq!(commands(dir), ["private"]);
}

#[test]
fn incidental_nested_ignore_rules_and_repository_boundaries_are_respected() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    write(dir, "package.json", "{}");
    write(dir, "apps/.gitignore", "ignored/\n");
    write(dir, "apps/.runuiignore", "private/\n");
    for name in ["ignored", "private", "kept", "checkout"] {
        package(dir, &format!("apps/{name}"), name);
    }
    write(dir, "apps/checkout/.git/HEAD", "ref: refs/heads/main\n");
    assert_eq!(commands(dir), ["kept"]);
}

#[test]
fn literal_missing_directories_and_glob_expansions_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    write(dir, "package.json", r#"{"workspaces":["future/deep/path/web", "apps/*"]}"#);
    let before = discover(&dir.to_string_lossy(), &dir.to_string_lossy());
    package(dir, "future/deep/path/web", "future");
    assert!(before.changed());
    let before = discover(&dir.to_string_lossy(), &dir.to_string_lossy());
    package(dir, "apps/web", "web");
    assert!(before.changed());
    assert_eq!(commands(dir), ["web", "future"]);
    let before = discover(&dir.to_string_lossy(), &dir.to_string_lossy());
    write(dir, "package.json", r#"{"workspaces":["future/deep/path/web"]}"#);
    assert!(before.changed());
    assert_eq!(commands(dir), ["future"]);
}

#[test]
fn dot_directories_require_explicit_patterns_and_unsafe_members_are_diagnosed() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    write(
        dir,
        "package.json",
        r#"{"workspaces":["packages/*", "packages/.kept", "../escape", "/absolute", "@(dynamic)", "linked"]}"#,
    );
    package(dir, "packages/.hidden", "hidden");
    package(dir, "packages/.kept", "kept");
    package(dir, "packages/web", "web");
    let other = tempfile::tempdir().unwrap();
    package(other.path(), "outside", "outside");
    std::os::unix::fs::symlink(other.path().join("outside"), dir.join("linked")).unwrap();
    let discovery = discover(&dir.to_string_lossy(), &dir.to_string_lossy());
    assert_eq!(commands(dir), ["kept", "web"]);
    assert!(discovery.diagnostics.len() >= 4, "{:?}", discovery.diagnostics);
    assert!(!discovery.complete());
}

#[test]
fn workspace_identity_stays_separate_from_nearest_project_and_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    write(dir, "package.json", r#"{"workspaces":["packages/*", "!packages/excluded"]}"#);
    package(dir, "packages/web", "web");
    package(dir, "packages/excluded", "excluded");
    package(dir, "standalone", "standalone");
    for folder in ["packages/excluded", "standalone"] {
        let project = dir.join(folder);
        assert_eq!(workspace_root(&project.to_string_lossy(), &dir.to_string_lossy()), project.to_string_lossy());
    }
    let web = dir.join("packages/web");
    assert_eq!(workspace_root(&web.to_string_lossy(), &dir.to_string_lossy()), dir.to_string_lossy());
    assert_eq!(crate::context_root(&web.to_string_lossy(), &dir.to_string_lossy()), dir.to_string_lossy());
    assert_eq!(workspace_root(&web.to_string_lossy(), &web.to_string_lossy()), web.to_string_lossy());
    write(dir, "packages/web/.git/HEAD", "ref: refs/heads/main\n");
    assert_eq!(crate::context_root(&web.to_string_lossy(), &dir.to_string_lossy()), web.to_string_lossy());
}

#[test]
fn malformed_manifests_keep_other_sources_and_report_partial_discovery() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    write(dir, "package.json", r#"{"workspaces": [42], "scripts":{"root":"true"}}"#);
    write(dir, "Makefile", "check:\n\ttrue\n");
    let discovery = discover(&dir.to_string_lossy(), &dir.to_string_lossy());
    assert!(commands(dir).contains(&"check".into()));
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("workspace declaration")));
}

#[test]
fn workspace_patterns_are_bounded_and_symlink_prefixes_are_not_followed() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let outside = tempfile::tempdir().unwrap();
    package(outside.path(), "secret/deep", "outside");
    std::os::unix::fs::symlink(outside.path(), dir.join("linked")).unwrap();
    write(dir, "package.json", r#"{"workspaces":["linked/secret/deep"]}"#);
    let discovery = discover(&dir.to_string_lossy(), &dir.to_string_lossy());
    assert!(commands(dir).is_empty());
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("symlink")));
    let patterns: Vec<_> = (0..300).map(|index| format!("packages/p{index}")).collect();
    write(dir, "package.json", &serde_json::json!({"workspaces":patterns}).to_string());
    let discovery = discover(&dir.to_string_lossy(), &dir.to_string_lossy());
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("pattern limit")));
}
