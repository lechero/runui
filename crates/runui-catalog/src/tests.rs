use super::*;
use serde_json::json;
use std::process::Command;

fn fixture() -> (tempfile::TempDir, Context) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project with spaces");
    fs::create_dir(&project).unwrap();
    let mut context = Context::new(project.to_str().unwrap(), project.to_str().unwrap());
    context.environment = Environment::default();
    context.environment.shell = vec!["/bin/sh".into(), "-c".into()];
    (temp, context)
}
fn write(context: &Context, name: &str, content: &str) {
    let path = Path::new(&context.project).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}
fn snapshot(context: &Context) -> Snapshot {
    Snapshot::build(
        context,
        &runui_project::discover(&context.project, &context.checkout),
        &[],
        &[],
        &usage::Data::default(),
    )
}
fn script(context: &Context, name: &str, body: &str) -> String {
    let path = Path::new(&context.project).join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn identity_is_stable_across_refresh_and_definition_edits() {
    let (_temp, context) = fixture();
    write(&context, "package.json", r#"{"scripts":{"test":"echo before"}}"#);
    let before = snapshot(&context);
    let entry = &before.entries()[0];
    assert!(entry.id.starts_with("cmd-v1-"));
    assert_eq!(entry.id.len(), 71);
    assert_eq!(snapshot(&context).entries()[0].id, entry.id);
    write(&context, "package.json", r#"{"scripts":{"test":"echo after"}}"#);
    let after = snapshot(&context);
    assert_eq!(after.entries()[0].id, entry.id);
    assert_ne!(after.entries()[0].fingerprint, entry.fingerprint);
    assert!(matches!(
        after.prepare(&entry.id, Some(&entry.fingerprint), &Inputs::default()),
        Err(PrepareError::Stale { .. })
    ));
    assert!(after.prepare(&entry.id, None, &Inputs::default()).is_ok());
    fs::remove_file(Path::new(&context.project).join("package.json")).unwrap();
    assert!(matches!(snapshot(&context).prepare(&entry.id, None, &Inputs::default()), Err(PrepareError::Unknown(_))));
}

#[test]
fn cargo_preserves_invocation_cwd_qualifies_manifests_and_keeps_native_identity() {
    let (_temp, mut context) = fixture();
    write(&context, "Cargo.toml", "[workspace]\nmembers=['crates/*']\n");
    for name in ["one", "two"] {
        write(&context, &format!("crates/{name}/Cargo.toml"), &format!("[package]\nname='{name}'\nedition='2024'\n"));
        write(&context, &format!("crates/{name}/src/main.rs"), "fn main() {}\n");
    }
    context.invocation_dir = canonical(&Path::new(&context.project).join("crates/one/src").to_string_lossy());
    let mut discovery = runui_project::discover(&context.project, &context.checkout);
    let before = Snapshot::build(&context, &discovery, &[], &[], &usage::Data::default());
    let mut query = Query::parse("source:cargo check").unwrap();
    query.commands();
    let hits = before.search(&query);
    assert_eq!(hits[0].entry.cargo.as_ref().unwrap().package.as_deref(), Some("one"));
    assert_eq!(hits[0].entry.directory, context.invocation_dir);
    assert_eq!(hits[0].entry.relative_source_directory(), "crates/one");
    let mut filtered = Query::parse("source:cargo dir:crates/two").unwrap();
    filtered.commands();
    assert!(
        before.search(&filtered).iter().all(|hit| hit.entry.cargo.as_ref().unwrap().package.as_deref() == Some("two"))
    );
    let run = before.entries().iter().find(|entry| entry.name == "run --bin one (one)").unwrap();
    let prepared = before
        .prepare(&run.id, None, &Inputs { args: vec!["--literal".into(), "space value".into()], ..Inputs::default() })
        .unwrap();
    assert_eq!(prepared.directory, context.invocation_dir);
    assert!(prepared.command.ends_with("-- --literal 'space value'"));
    for file in &mut discovery.files {
        for command in &mut file.commands {
            command.name = format!("A clearer label for {}", command.name);
        }
    }
    let renamed = Snapshot::build(&context, &discovery, &[], &[], &usage::Data::default());
    assert!(renamed.get(&run.id).is_some());
    assert_eq!(renamed.get(&run.id).unwrap().fingerprint, run.fingerprint);
    write(&context, "crates/one/src/.cargo/config.toml", "[build]\ntarget-dir='elsewhere'\n");
    let updated = snapshot(&context);
    assert_ne!(updated.get(&run.id).unwrap().fingerprint, run.fingerprint);
    assert!(matches!(
        updated.prepare(&run.id, Some(&run.fingerprint), &Inputs::default()),
        Err(PrepareError::Stale { .. })
    ));
    context.invocation_dir = canonical("/private/tmp/outside-checkout");
    assert!(
        snapshot(&context)
            .entries()
            .iter()
            .filter(|entry| entry.source == "cargo")
            .all(|entry| entry.directory == context.project)
    );
}

#[test]
fn cargo_workspace_defaults_sort_before_all_members_without_changing_explicit_selection() {
    let (_temp, context) = fixture();
    write(&context, "Cargo.toml", "[workspace]\nmembers=['crate']\n");
    write(&context, "crate/Cargo.toml", "[package]\nname='crate'\nedition='2024'\n");
    write(&context, "crate/src/lib.rs", "pub fn check() {}\n");
    let snapshot = snapshot(&context);
    let mut query = Query::parse("source:cargo check").unwrap();
    query.commands();
    assert_eq!(
        snapshot.search(&query)[0].entry.cargo.as_ref().unwrap().scope,
        runui_project::CargoScope::DefaultMembers
    );
    query = Query::parse("source:cargo all").unwrap();
    query.commands();
    assert_eq!(snapshot.search(&query)[0].entry.cargo.as_ref().unwrap().scope, runui_project::CargoScope::AllMembers);
    let test = snapshot.entries().iter().find(|entry| entry.name == "test (default members)").unwrap();
    assert!(
        snapshot
            .prepare(&test.id, None, &Inputs { args: vec!["--nocapture".into()], ..Inputs::default() })
            .unwrap()
            .command
            .ends_with("-- --nocapture")
    );
}

#[test]
fn worktrees_and_same_names_in_different_folders_keep_distinct_identities() {
    let (_temp, context) = fixture();
    write(&context, "package.json", r#"{"scripts":{"test":"true"}}"#);
    write(&context, "apps/web/package.json", r#"{"scripts":{"test":"true"}}"#);
    let first = snapshot(&context);
    assert_eq!(first.entries().len(), 2);
    assert_ne!(first.entries()[0].id, first.entries()[1].id);
    let (_other, other) = fixture();
    write(&other, "package.json", r#"{"scripts":{"test":"true"}}"#);
    assert_ne!(first.entries()[0].id, snapshot(&other).entries()[0].id);
}

#[test]
fn package_and_workspace_scopes_use_the_same_checkout_qualified_id() {
    let (_temp, context) = fixture();
    write(&context, "package.json", "{}");
    write(&context, "apps/web/package.json", r#"{"scripts":{"test":"true"}}"#);
    let workspace = snapshot(&context);
    let package = Context::new(&Path::new(&context.project).join("apps/web").to_string_lossy(), &context.checkout);
    let package = snapshot(&package);
    assert_eq!(workspace.entries()[0].id, package.entries()[0].id);
}

#[test]
fn workspace_favorites_merge_aliases_and_nearby_packages_break_equal_search_ties() {
    let (_temp, mut context) = fixture();
    write(&context, "package.json", r#"{"workspaces":["packages/*"],"scripts":{"test":"true"}}"#);
    write(&context, "packages/web/package.json", r#"{"scripts":{"test":"true"}}"#);
    context.invocation_dir = Path::new(&context.project).join("packages/web").to_string_lossy().into_owned();
    let mut favorites =
        runui_favorites::Store::open(&Path::new(&context.project).join("favorites.json").to_string_lossy()).unwrap();
    favorites
        .add(
            &context.project,
            Favorite {
                name: "root alias".into(),
                command: "echo saved".into(),
                dir: "packages/web".into(),
                ..Favorite::default()
            },
        )
        .unwrap();
    favorites
        .add(
            &context.invocation_dir,
            Favorite { name: "package alias".into(), command: "echo saved".into(), ..Favorite::default() },
        )
        .unwrap();
    let projection = favorites.for_tree(&context.project);
    let snapshot = Snapshot::build(
        &context,
        &runui_project::discover(&context.project, &context.checkout),
        &projection,
        &[],
        &usage::Data::default(),
    );
    let hits = snapshot.search(&Query::parse("test").unwrap());
    assert_eq!(hits[0].entry.directory, context.invocation_dir);
    let saved = snapshot.search(&Query::parse("source:shell").unwrap());
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].entry.favorite_names.len(), 2);
    assert!(snapshot.prepare(&saved[0].entry.id, None, &Inputs::default()).is_ok());
    assert_eq!(favorites.for_dir(&context.project)[0].dir, "packages/web");
    assert_eq!(favorites.for_dir(&context.invocation_dir)[0].dir, "");
}

#[test]
fn include_provenance_and_execution_fingerprints_include_hidden_definitions() {
    let (_temp, context) = fixture();
    write(&context, "Taskfile.yml", "version: '3'\nincludes:\n  one: .tasks/shared.yml\n  two: .tasks/shared.yml\n");
    write(&context, ".tasks/shared.yml", "version: '3'\ntasks:\n  build: {cmds: ['echo before']}\n");
    let before = snapshot(&context);
    assert_eq!(before.entries().len(), 2);
    let entry = &before.entries()[0];
    assert!(entry.entry_file.as_ref().unwrap().ends_with("/Taskfile.yml"));
    assert!(entry.defining_file.as_ref().unwrap().ends_with("/.tasks/shared.yml"));
    assert_eq!(entry.directory, context.project);
    assert_eq!(entry.defining_line, Some(3));
    write(&context, ".tasks/shared.yml", "version: '3'\ntasks:\n  build: {cmds: ['echo after']}\n");
    let after = snapshot(&context);
    assert_eq!(after.entries()[0].id, entry.id);
    assert_ne!(after.entries()[0].fingerprint, entry.fingerprint);
}

#[test]
fn default_preparation_matches_the_command_preview_for_every_provider() {
    let (_temp, context) = fixture();
    write(&context, "Taskfile.yml", "version: '3'\ntasks:\n  odd name: {cmds: ['true']}\n");
    write(&context, "package.json", r#"{"scripts":{"odd'name":"true"}}"#);
    write(&context, "Makefile", "test:\n\ttrue\n");
    write(&context, "compose.yaml", "services:\n  odd name: {}\n");
    write(&context, "pom.xml", "<project><profiles><profile><id>odd name</id></profile></profiles></project>");
    let snapshot = snapshot(&context);
    assert!(snapshot.entries().len() > 10);
    for entry in snapshot.entries() {
        let prepared = snapshot.prepare(&entry.id, Some(&entry.fingerprint), &Inputs::default()).unwrap();
        assert_eq!(entry.command, prepared.command, "{}: {}", entry.source, entry.name);
    }
}

#[test]
fn favorite_badges_merge_without_losing_definition_origins_or_custom_commands() {
    let (_temp, context) = fixture();
    write(&context, "package.json", r#"{"scripts":{"test":"true"}}"#);
    let favorites = vec![
        Favorite {
            name: "Fast checks".into(),
            source: "npm".into(),
            command: "npm run test".into(),
            ..Favorite::default()
        },
        Favorite { name: "Custom".into(), command: "echo custom".into(), ..Favorite::default() },
        Favorite {
            name: "Elsewhere".into(),
            command: "npm run test".into(),
            dir: "different".into(),
            ..Favorite::default()
        },
    ];
    let entries = Snapshot::build(
        &context,
        &runui_project::discover(&context.project, &context.checkout),
        &favorites,
        &[],
        &usage::Data::default(),
    );
    assert_eq!(entries.entries().len(), 3);
    assert!(entries.entries()[0].favorite);
    assert_eq!(entries.entries()[0].name, "test");
    assert_eq!(entries.search(&Query::parse("fast checks").unwrap())[0].entry.name, "test");
    assert_eq!(entries.entries().iter().filter(|entry| entry.kind == Kind::Favorite).count(), 2);
}

#[test]
fn ranking_and_facets_search_all_sources_deterministically() {
    let (_temp, context) = fixture();
    write(&context, "Taskfile.yml", "version: '3'\ntasks:\n  deploy: {desc: test everything}\n");
    write(&context, "package.json", r#"{"scripts":{"test":"check"}}"#);
    write(&context, "apps/web/Makefile", "test-web: ## test web\n");
    let snapshot = snapshot(&context);
    let query = Query::parse("test").unwrap();
    let hits = snapshot.search(&query);
    assert_eq!(hits[0].entry.name, "test");
    assert!(hits.iter().any(|hit| hit.entry.source == "task"));
    assert!(hits.iter().any(|hit| hit.entry.source == "make"));
    assert_eq!(snapshot.search(&Query::parse("source:make dir:apps/web tst").unwrap())[0].entry.name, "test-web");
    let ids: Vec<_> = hits.iter().map(|hit| &hit.entry.id).collect();
    assert_eq!(ids, snapshot.search(&query).iter().map(|hit| &hit.entry.id).collect::<Vec<_>>());
    let mut page = query.clone();
    page.limit = 1;
    page.offset = 1;
    assert_eq!(snapshot.search(&page)[0].entry.id, hits[1].entry.id);
}

#[test]
fn literal_colons_quotes_unicode_and_invalid_facets_have_defined_behavior() {
    let (_temp, context) = fixture();
    write(&context, "package.json", r#"{"scripts":{"source:npm":"true","café":"true","docs:serve":"true"}}"#);
    let snapshot = snapshot(&context);
    assert_eq!(snapshot.search(&Query::parse(r"source\:npm").unwrap())[0].entry.name, "source:npm");
    assert_eq!(snapshot.search(&Query::parse("'source:npm'").unwrap())[0].entry.name, "source:npm");
    assert_eq!(snapshot.search(&Query::parse("CAFÉ").unwrap())[0].entry.name, "café");
    assert_eq!(snapshot.search(&Query::parse("docs:serve").unwrap())[0].entry.name, "docs:serve");
    assert!(Query::parse("source:").is_err());
    assert!(Query::parse("'unclosed").is_err());
}

#[test]
fn arguments_round_trip_literally_with_the_exact_working_directory() {
    let (_temp, mut context) = fixture();
    let npm = script(&context, "npm", "printf '%s\\0' \"$PWD\" \"$@\"");
    context.environment.path = vec![Path::new(&npm).parent().unwrap().to_path_buf()];
    write(&context, "package.json", r#"{"scripts":{"odd'$(touch NEVER)":"true"}}"#);
    let snapshot = snapshot(&context);
    let entry = &snapshot.entries()[0];
    assert_eq!(entry.availability, Availability::OnPath);
    let args = vec!["spaces here", "'\"$();`touch NEVER`", "line\nbreak", "", "☕"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let prepared = snapshot
        .prepare(&entry.id, Some(&entry.fingerprint), &Inputs { args: args.clone(), ..Inputs::default() })
        .unwrap();
    let output = Command::new("/bin/sh")
        .args(["-c", &prepared.command])
        .current_dir(&prepared.directory)
        .env("PATH", &context.project)
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    let mut actual: Vec<_> =
        output.stdout.split(|byte| *byte == 0).map(|part| String::from_utf8(part.into()).unwrap()).collect();
    assert_eq!(actual.pop(), Some(String::new()));
    let expected = vec![
        context.project.clone(),
        "run".into(),
        entry.name.clone(),
        "--".into(),
        args[0].clone(),
        args[1].clone(),
        args[2].clone(),
        args[3].clone(),
        args[4].clone(),
    ];
    assert_eq!(actual, expected);
    assert!(!Path::new(&context.project).join("NEVER").exists());
}

#[test]
fn composer_aliases_hooks_readiness_and_timeout_keep_one_native_definition() {
    let (_temp, mut context) = fixture();
    write(
        &context,
        "composer.json",
        r#"{"scripts":{"check":"@php check.php","pre-deploy":"true","post-install-cmd":"true"},"scripts-aliases":{"check":["verify"]}}"#,
    );
    let composer = script(&context, "composer", "printf '%s\\0' \"$COMPOSER\" \"$PWD\" \"$@\"");
    context.environment.path = vec![Path::new(&composer).parent().unwrap().to_path_buf()];
    let before = snapshot(&context);
    let mut query = Query::parse("source:composer").unwrap();
    query.commands();
    assert_eq!(before.search(&query).len(), 2);
    let mut hook = Query::parse("source:composer kind:hook").unwrap();
    hook.commands();
    assert_eq!(before.search(&hook)[0].entry.name, "post-install-cmd");
    let mut alias = Query::parse("source:composer verify").unwrap();
    alias.commands();
    let entry = before.search(&alias)[0].entry;
    assert_eq!(entry.name, "check");
    assert_eq!(entry.aliases, ["verify"]);
    assert_eq!(entry.availability, Availability::Missing);
    assert_eq!(entry.requirements[0].availability, Availability::OnPath);
    assert_eq!(entry.requirements[1].program, "php");
    assert_eq!(entry.requirements[1].availability, Availability::Missing);
    let args = vec!["--flag", "a space", "'\"$();`touch NEVER`", "", "line\nbreak", "☕"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let prepared = before
        .prepare(&entry.id, None, &Inputs { args: args.clone(), timeout_seconds: Some(0), ..Inputs::default() })
        .unwrap();
    let output = Command::new("/bin/sh")
        .args(["-c", &prepared.command])
        .current_dir(&prepared.directory)
        .env("PATH", format!("{}:/usr/bin:/bin", context.project))
        .env("COMPOSER", "wrong.json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let values: Vec<_> =
        output.stdout.split(|byte| *byte == 0).map(|part| String::from_utf8(part.into()).unwrap()).collect();
    let mut expected = vec![
        canonical(&Path::new(&context.project).join("composer.json").to_string_lossy()),
        context.project.clone(),
        "run-script".into(),
        "--timeout=0".into(),
        "--".into(),
        "check".into(),
    ];
    expected.extend(args);
    expected.push(String::new());
    assert_eq!(values, expected);
    assert!(!Path::new(&context.project).join("NEVER").exists());
    script(&context, "php", "exit 0");
    let after = snapshot(&context);
    let updated = after.get(&entry.id).unwrap();
    assert_eq!(updated.availability, Availability::OnPath);
    assert_ne!(updated.fingerprint, entry.fingerprint);
    assert!(matches!(
        after.prepare(&entry.id, Some(&entry.fingerprint), &Inputs::default()),
        Err(PrepareError::Stale { .. })
    ));
    context.environment.project_wrapper = true;
    assert_eq!(snapshot(&context).get(&entry.id).unwrap().availability, Availability::ProjectEnvironment);
    write(&context, "Makefile", "test:\n");
    let snapshot = snapshot(&context);
    let make = snapshot.entries().iter().find(|entry| entry.source == "make").unwrap();
    assert!(
        snapshot
            .prepare(&make.id, None, &Inputs { timeout_seconds: Some(5), ..Inputs::default() })
            .unwrap_err()
            .to_string()
            .contains("Composer")
    );
}

#[test]
fn raw_shell_arguments_are_positional_instead_of_attaching_to_a_pipeline() {
    let (_temp, context) = fixture();
    let favorite = Favorite { command: "printf '%s\\0' \"$1\" \"$2\"".into(), ..Favorite::default() };
    let snapshot = Snapshot::build(&context, &Discovery::default(), &[favorite], &[], &usage::Data::default());
    let prepared = snapshot
        .prepare(
            &snapshot.entries()[0].id,
            None,
            &Inputs { args: vec!["a; touch NEVER".into(), "".into()], ..Inputs::default() },
        )
        .unwrap();
    let output =
        Command::new("/bin/sh").args(["-c", &prepared.command]).current_dir(&prepared.directory).output().unwrap();
    assert_eq!(output.stdout, b"a; touch NEVER\0\0");
    assert!(!Path::new(&context.project).join("NEVER").exists());
}

#[test]
fn variables_and_provider_separators_are_not_inferred_from_shell_text() {
    let (_temp, context) = fixture();
    write(&context, "Taskfile.yml", "version: '3'\ntasks:\n  test: echo test\n");
    write(&context, "Makefile", "test:\n");
    write(&context, "pom.xml", "<project/>");
    let snapshot = snapshot(&context);
    let inputs = Inputs {
        args: vec!["arg value".into()],
        variables: BTreeMap::from([("NAME".into(), "x;echo BAD".into())]),
        ..Inputs::default()
    };
    for entry in snapshot.entries() {
        let prepared = snapshot.prepare(&entry.id, None, &inputs).unwrap();
        assert!(prepared.command.contains("'NAME=x;echo BAD'") || prepared.command.contains("'-DNAME=x;echo BAD'"));
        if entry.source == "task" {
            assert!(prepared.command.contains(" -- 'arg value'"));
        }
    }
    let invalid = Inputs { variables: BTreeMap::from([("-bad".into(), "value".into())]), ..Inputs::default() };
    assert!(snapshot.prepare(&snapshot.entries()[0].id, None, &invalid).is_err());
    let nul = Inputs { args: vec!["bad\0value".into()], ..Inputs::default() };
    assert!(snapshot.prepare(&snapshot.entries()[0].id, None, &nul).is_err());
}

#[test]
fn runs_are_focus_targets_and_recent_invocations_remain_launchable() {
    let (_temp, context) = fixture();
    let run:Run=serde_json::from_value(json!({"id":"old","dir":context.project,"command":"echo done","started":Utc::now().to_rfc3339(),"ended":Utc::now().to_rfc3339(),"exit":0})).unwrap();
    let snapshot = Snapshot::build(&context, &Discovery::default(), &[], &[run], &usage::Data::default());
    let run = &snapshot.entries()[0];
    assert_eq!(snapshot.search(&Query::parse("kind:run state:completed").unwrap()).len(), 1);
    assert_eq!(snapshot.prepare(&run.id, None, &Inputs::default()).unwrap_err(), PrepareError::RunSelection);
}

#[test]
fn discovery_failures_are_visible_without_erasing_valid_source_entries() {
    let (_temp, context) = fixture();
    write(&context, "package.json", "{");
    write(&context, "Makefile", "build:\n");
    let snapshot = snapshot(&context);
    assert_eq!(snapshot.entries().len(), 1);
    assert_eq!(snapshot.diagnostics.len(), 1);
    assert_eq!(snapshot.entries()[0].availability, Availability::Missing);
}

#[test]
fn symlinked_entry_points_keep_ids_and_fingerprints_for_the_same_checkout() {
    let (temp, context) = fixture();
    write(
        &context,
        "Taskfile.yml",
        "version: '3'\nincludes:\n  optional:\n    taskfile: .tasks/missing\n    optional: true\ntasks:\n  build: echo build\n",
    );
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&context.project, &alias).unwrap();
    let aliased = Context::new(&alias.to_string_lossy(), &alias.to_string_lossy());
    let indirect = Snapshot::build(
        &aliased,
        &runui_project::discover(&alias.to_string_lossy(), &alias.to_string_lossy()),
        &[],
        &[],
        &usage::Data::default(),
    );
    let direct = snapshot(&context);
    assert_eq!(direct.entries()[0].id, indirect.entries()[0].id);
    assert_eq!(direct.entries()[0].fingerprint, indirect.entries()[0].fingerprint);
}

#[test]
fn quoted_facet_values_keep_directories_with_spaces_searchable() {
    let (_temp, context) = fixture();
    write(&context, "package.json", "{}");
    write(&context, "apps/web space/Makefile", "test:\n");
    let snapshot = snapshot(&context);
    assert_eq!(snapshot.search(&Query::parse("dir:\"apps/web space\"").unwrap()).len(), 1);
}

#[test]
fn generated_files_in_an_include_directory_do_not_change_a_definition_fingerprint() {
    let (_temp, context) = fixture();
    write(&context, "Taskfile.yml", "version: '3'\nincludes:\n  docs: docs\n");
    write(&context, "docs/Taskfile.yml", "version: '3'\ntasks:\n  build: echo build\n");
    let before = snapshot(&context);
    write(&context, "docs/generated.html", "output");
    let after = snapshot(&context);
    assert_eq!(before.entries()[0].fingerprint, after.entries()[0].fingerprint);
}

#[test]
fn version_one_identity_encoding_is_fixed_and_length_delimited() {
    assert_eq!(
        digest(&["checkout", "task", "Taskfile.yml", "build", "", "src"]),
        "e481b433257dd829f94c2a3be00e09bbafaee7fd376518fe391283d1923b9b44"
    );
    assert_ne!(digest(&["a", "bc"]), digest(&["ab", "c"]));
}

#[test]
fn provider_variants_do_not_collide_even_when_labels_and_arguments_match() {
    let (_temp, context) = fixture();
    write(&context, "package.json", r#"{"scripts":{"build":"true"}}"#);
    let mut discovery = runui_project::discover(&context.project, &context.checkout);
    let first = Snapshot::build(&context, &discovery, &[], &[], &usage::Data::default());
    discovery.files[0].commands[0].variant = "another-provider-context".into();
    let second = Snapshot::build(&context, &discovery, &[], &[], &usage::Data::default());
    assert_ne!(first.entries()[0].id, second.entries()[0].id);
}

#[test]
fn ambiguous_discovery_never_silently_prepares_the_first_definition() {
    let (_temp, context) = fixture();
    write(&context, "Taskfile.yml", "version: '3'\ntasks:\n  build: echo one\n");
    let mut discovery = runui_project::discover(&context.project, &context.checkout);
    let duplicate = discovery.files[0].commands[0].clone();
    discovery.files[0].commands.push(duplicate);
    let snapshot = Snapshot::build(&context, &discovery, &[], &[], &usage::Data::default());
    assert!(snapshot.diagnostics.iter().any(|problem| problem.kind == "definition"));
    assert!(matches!(
        snapshot.prepare(&snapshot.entries()[0].id, None, &Inputs::default()),
        Err(PrepareError::Ambiguous(_))
    ));
}
