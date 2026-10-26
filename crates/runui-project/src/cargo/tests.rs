use super::*;
use crate::discover;
use std::collections::BTreeSet;
use std::fs;

fn write(root: &Path, name: &str, body: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}
fn package(root: &Path, folder: &str, name: &str, extra: &str) {
    write(
        root,
        &format!("{folder}/Cargo.toml"),
        &format!("[package]\nname={name:?}\nversion='0.1.0'\nedition='2024'\n{extra}"),
    );
    write(root, &format!("{folder}/src/lib.rs"), "pub fn check() {}\n");
}
fn scan(root: &Path) -> crate::Discovery {
    discover(&root.to_string_lossy(), &root.to_string_lossy())
}
fn packages(discovery: &crate::Discovery) -> BTreeSet<String> {
    discovery
        .files
        .iter()
        .flat_map(|file| &file.commands)
        .filter_map(|command| command.cargo.as_ref()?.package.clone())
        .collect()
}

#[test]
fn virtual_workspaces_resolve_deep_default_excluded_and_automatic_path_members() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers=['crates/cli','deep/a/b/c/*']\nexclude=['deep/a/b/c/old','engine/helper']\ndefault-members=['crates/cli']\n[workspace.dependencies]\nengine={path='engine/core'}\n",
    );
    package(root, "crates/cli", "cli", "[dependencies]\nengine.workspace=true\n");
    package(root, "engine/core", "engine", "[build-dependencies]\nhelper={path='../helper'}\n");
    package(root, "engine/helper", "helper", "");
    package(root, "deep/a/b/c/lib", "deep", "");
    package(root, "deep/a/b/c/old", "old", "");
    package(root, "incidental", "unlisted", "");
    write(root, ".gitignore", "engine/\n");
    let discovery = scan(root);
    assert!(discovery.complete(), "{:?}", discovery.diagnostics);
    assert_eq!(packages(&discovery), BTreeSet::from(["cli".into(), "engine".into(), "deep".into()]));
    let root_file = discovery.files.iter().find(|file| file.source == Source::Cargo && file.rel.is_empty()).unwrap();
    assert_eq!(root_file.commands.len(), 6);
    let default = &root_file.commands[0];
    assert_eq!(default.cargo.as_ref().unwrap().scope, CargoScope::DefaultMembers);
    assert_eq!(default.cargo.as_ref().unwrap().default_member_paths, ["crates/cli"]);
    assert!(!default.argv.iter().any(|arg| arg == "--workspace"));
    assert!(root_file.commands[3].argv.iter().any(|arg| arg == "--workspace"));
    assert!(!root_file.commands.iter().any(|command| command.argv.iter().any(|arg| arg == "--all-features")));
    assert!(
        discovery
            .notices
            .iter()
            .any(|notice| notice.path.contains("engine/core") && notice.message.contains("despite"))
    );
    assert_eq!(
        crate::workspace_root(&root.join("engine/core").to_string_lossy(), &root.to_string_lossy()),
        root.to_string_lossy()
    );
    assert!(!discovery.changed());
    write(root, "engine/core/src/bin/new.rs", "fn main() {}\n");
    assert!(discovery.changed());
    assert!(
        scan(root).files.iter().flat_map(|file| &file.commands).any(|command| command.name == "run --bin new (engine)")
    );
}

#[test]
fn target_kinds_default_run_features_and_library_examples_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    package(
        root,
        ".",
        "sample",
        r#"
default-run='server'
[features]
default=['common']
common=[]
server=[]
[[bin]]
name='server'
path='src/bin/server/main.rs'
required-features=['server']
[[example]]
name='library'
crate-type=['staticlib']
"#,
    );
    for (name, body) in [
        ("src/main.rs", "fn main() {}"),
        ("src/bin/server/main.rs", "fn main() {}"),
        ("src/bin/other.rs", "fn main() {}"),
        ("examples/library.rs", "pub fn example() {}"),
        ("examples/demo/main.rs", "fn main() {}"),
        ("tests/integration.rs", "#[test] fn check() {}"),
        ("benches/measure.rs", "fn main() {}"),
    ] {
        write(root, name, body);
    }
    let discovery = scan(root);
    assert!(discovery.complete(), "{:?}", discovery.diagnostics);
    let commands = &discovery.files[0].commands;
    assert!(commands.iter().all(|command| command.generated));
    assert_eq!(commands.len(), 11);
    let server = commands.iter().find(|command| command.name == "run --bin server (sample)").unwrap();
    assert!(server.args_separator);
    assert!(server.argv.ends_with(&["--features".into(), "server".into()]));
    let details = server.cargo.as_ref().unwrap();
    assert_eq!(details.available_features, ["common", "default", "server"]);
    assert_eq!(details.default_features, ["common"]);
    assert!(details.target.as_ref().unwrap().default_run);
    assert_eq!(details.target.as_ref().unwrap().required_features, ["server"]);
    let library = commands.iter().find(|command| command.name == "build --example library (sample)").unwrap();
    assert!(!library.args_separator);
    assert!(!library.cargo.as_ref().unwrap().target.as_ref().unwrap().runnable);
    assert!(!commands.iter().any(|command| command.name.starts_with("run --example library")));
    assert!(commands.iter().any(|command| command.name == "test --test integration (sample)"));
    assert!(commands.iter().any(|command| command.name == "bench --bench measure (sample)"));
}

#[test]
fn library_only_and_manual_targets_do_not_invent_runnable_binaries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    package(root, ".", "lib_only", "");
    let discovery = scan(root);
    assert_eq!(discovery.files[0].commands.len(), 4);
    assert!(!discovery.files[0].commands.iter().any(|command| command.name.starts_with("run")));
    write(
        root,
        "Cargo.toml",
        "[package]\nname='renamed'\nedition='2024'\nautobins=false\n[[bin]]\nname='explicit'\npath='src/main.rs'\n",
    );
    write(root, "src/main.rs", "fn main() {}\n");
    write(root, "src/bin/not-a-target.rs", "fn main() {}\n");
    let discovery = scan(root);
    assert!(discovery.complete());
    let binaries: Vec<_> = discovery.files[0]
        .commands
        .iter()
        .filter_map(|command| command.cargo.as_ref()?.target.as_ref())
        .filter(|target| target.kind == TargetKind::Binary)
        .collect();
    assert_eq!(binaries.len(), 1);
    assert_eq!(binaries[0].name, "explicit");
    assert!(binaries[0].default_run);
}

#[test]
fn ambiguous_inferred_targets_are_diagnosed_and_not_offered_as_named_runs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    package(root, ".", "ambiguous", "");
    write(root, "src/bin/duplicate.rs", "fn main() {}\n");
    write(root, "src/bin/duplicate/main.rs", "fn main() {}\n");
    let discovery = scan(root);
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|diagnostic| diagnostic.message.contains("multiple inferred binary")));
    assert!(!discovery.files.iter().flat_map(|file| &file.commands).any(|command| command.name.starts_with("run")));
    assert!(
        discovery.files.iter().flat_map(|file| &file.commands).any(|command| command.name == "test --lib ambiguous")
    );
}

#[test]
fn root_package_defaults_remain_distinct_from_all_members_and_nested_source_filters() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    package(root, ".", "root", "[workspace]\nmembers=['crates/*']\n");
    package(root, "crates/member", "member", "");
    write(root, "package.json", r#"{"workspaces":["web/*","!crates/*"]}"#);
    write(root, "web/app/package.json", r#"{"scripts":{"test":"true"}}"#);
    let discovery = scan(root);
    assert!(discovery.complete(), "{:?}", discovery.diagnostics);
    assert_eq!(packages(&discovery), BTreeSet::from(["root".into(), "member".into()]));
    let file = discovery.files.iter().find(|file| file.source == Source::Cargo && file.rel.is_empty()).unwrap();
    assert_eq!(file.commands[0].cargo.as_ref().unwrap().default_member_paths, ["."]);
    assert!(file.commands.iter().any(|command| command.cargo.as_ref().unwrap().scope == CargoScope::Package));
    assert!(discovery.files.iter().any(|file| file.source == Source::Npm));
}

#[test]
fn malformed_cargo_keeps_other_sources_and_never_runs_build_scripts_or_native_tools() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Cargo.toml", "[package\nname='bad'\n");
    write(root, "Makefile", "check:\n\ttrue\n");
    write(root, "build.rs", "fn main() {std::fs::write(\"MUST_NOT_RUN\",\"oops\").unwrap();}\n");
    let discovery = scan(root);
    assert!(!discovery.complete());
    assert!(discovery.files.iter().any(|file| file.source == Source::Cargo && file.err.is_some()));
    assert!(discovery.files.iter().any(|file| file.source == Source::Make && !file.commands.is_empty()));
    assert!(!root.join("MUST_NOT_RUN").exists());
    assert!(!root.join("target").exists());
    assert!(!root.join("Cargo.lock").exists());
}

#[test]
fn path_dependencies_honor_local_ignore_symlink_and_checkout_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    package(
        root,
        ".",
        "root",
        "[workspace]\n[dependencies]\nhidden={path='hidden/deep/member'}\nexternal={path='../outside'}\nlinked={path='link'}\n",
    );
    package(root, "hidden/deep/member", "hidden", "");
    write(root, ".runuiignore", "hidden/\n");
    std::os::unix::fs::symlink(root.join("hidden/deep/member"), root.join("link")).unwrap();
    let discovery = scan(root);
    assert_eq!(packages(&discovery), BTreeSet::from(["root".into()]));
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|diagnostic| diagnostic.message.contains("outside")));
    assert!(discovery.diagnostics.iter().any(|diagnostic| diagnostic.message.contains("symlink")));
}

#[test]
fn cargo_recognizes_this_repositories_current_crates_without_native_metadata() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let discovery = scan(root);
    assert!(
        discovery.diagnostics.iter().all(|diagnostic| diagnostic.source != Some(Source::Cargo)),
        "{:?}",
        discovery.diagnostics
    );
    assert!(discovery.files.iter().filter(|file| file.source == Source::Cargo).all(|file| file.err.is_none()));
    assert_eq!(
        packages(&discovery),
        BTreeSet::from(
            [
                "runui",
                "runui-agents",
                "runui-catalog",
                "runui-core",
                "runui-favorites",
                "runui-project",
                "runui-runs",
                "runui-tea",
                "runui-tui"
            ]
            .map(str::to_owned)
        )
    );
}

#[test]
fn nested_workspaces_expand_from_polyglot_checkouts_independently_of_node_membership() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "package.json", r#"{"workspaces":["apps/*"]}"#);
    write(root, "rust/workspace/Cargo.toml", "[workspace]\nmembers=['deep/a/b/c/member']\n");
    package(root, "rust/workspace/deep/a/b/c/member", "member", "");
    write(root, "rust/workspace/Makefile", "hidden:\n\ttrue\n");
    let discovery = scan(root);
    assert_eq!(packages(&discovery), BTreeSet::from(["member".into()]));
    let member =
        discovery.files.iter().find(|file| file.rel.ends_with("/member") && file.source == Source::Cargo).unwrap();
    assert_eq!(member.rel, "rust/workspace/deep/a/b/c/member");
    assert_eq!(member.rel_path(), "rust/workspace/deep/a/b/c/member/Cargo.toml");
    assert!(discovery.files.iter().any(|file| file.source == Source::Make));
}
