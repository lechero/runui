use super::*;
use std::time::{Duration, SystemTime};

fn write(dir: &str, name: &str, body: &str) {
    let path = filepath::join(&[dir, name]);
    fs::create_dir_all(filepath::dir(&path)).unwrap();
    fs::write(path, body).unwrap();
}

fn temporary() -> (tempfile::TempDir, String) {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_str().unwrap().to_string();
    (temp, dir)
}

fn command_names(discovery: &Discovery) -> Vec<&str> {
    discovery.files.iter().flat_map(|file| file.commands.iter().map(|command| command.name.as_str())).collect()
}

fn edit(dir: &str, name: &str, body: &str) {
    write(dir, name, body);
    fs::File::options()
        .write(true)
        .open(filepath::join(&[dir, name]))
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
}

#[test]
fn a_shared_taskfile_expands_in_each_namespace() {
    let (_temp, dir) = temporary();
    write(&dir, "Taskfile.yml", "version: '3'\nincludes:\n  one: .tasks/shared.yml\n  two: .tasks/shared.yml\n");
    write(&dir, ".tasks/shared.yml", "version: '3'\ntasks:\n  build: {desc: Build}\n");
    let discovery = discover(&dir, &dir);
    assert!(discovery.complete(), "{:?}", discovery.diagnostics);
    assert_eq!(command_names(&discovery), ["one:build", "two:build"]);
    assert!(!discovery.changed());
}

#[test]
fn cancelled_discovery_does_not_return_usable_commands_as_a_complete_scan() {
    let (_temp, dir) = temporary();
    write(&dir, "Makefile", "test:\n\ttrue\n");
    let discovery = discover_cancellable(&dir, &dir, Arc::new(AtomicBool::new(true)));
    assert!(discovery.files.is_empty());
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|item| item.message.contains("cancelled")));
}

#[test]
fn cycles_keep_the_entry_file_and_report_partial_discovery() {
    let (_temp, dir) = temporary();
    write(&dir, "Taskfile.yml", "version: '3'\nincludes:\n  b: b/Taskfile.yml\ntasks:\n  a: echo a\n");
    write(&dir, "b/Taskfile.yml", "version: '3'\nincludes:\n  back: ../Taskfile.yml\ntasks:\n  x: echo x\n");
    let discovery = discover(&dir, &dir);
    assert_eq!(discovery.files.len(), 1);
    assert_eq!(command_names(&discovery), ["a", "b:x"]);
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("cycle")));
}

#[test]
fn cycles_through_symlinks_also_terminate() {
    let (_temp, dir) = temporary();
    write(&dir, "Taskfile.yml", "version: '3'\nincludes:\n  again: alias.yml\ntasks:\n  build: echo built\n");
    std::os::unix::fs::symlink("Taskfile.yml", filepath::join(&[&dir, "alias.yml"])).unwrap();
    let discovery = discover(&dir, &dir);
    assert_eq!(command_names(&discovery), ["build"]);
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("cycle")));
}

#[test]
fn hidden_include_edits_refresh_the_parent_commands() {
    let (_temp, dir) = temporary();
    write(&dir, "Taskfile.yml", "version: '3'\nincludes:\n  docs: .tasks/docs.yml\n");
    write(&dir, ".tasks/docs.yml", "version: '3'\ntasks:\n  serve: {desc: Before}\n");
    let before = discover(&dir, &dir);
    assert!(!before.changed());
    edit(&dir, ".tasks/docs.yml", "version: '3'\ntasks:\n  serve: {desc: After}\n");
    assert!(before.changed());
    let after = discover(&dir, &dir);
    assert_eq!(after.files[0].commands[0].desc, "After");
    assert!(!after.changed());
}

#[test]
fn an_optional_include_can_appear_as_a_directory() {
    let (_temp, dir) = temporary();
    write(&dir, "Taskfile.yml", "version: '3'\nincludes:\n  docs:\n    taskfile: .tasks/docs\n    optional: true\n");
    let before = discover(&dir, &dir);
    assert!(before.complete());
    write(&dir, ".tasks/docs/Taskfile.yaml", "version: '3'\ntasks:\n  serve: echo ready\n");
    assert!(before.changed());
    let after = discover(&dir, &dir);
    assert_eq!(command_names(&after), ["docs:serve"]);
    fs::remove_file(filepath::join(&[&dir, ".tasks/docs/Taskfile.yaml"])).unwrap();
    assert!(after.changed());
}

#[test]
fn newly_created_nested_manifests_and_directories_refresh() {
    let (_temp, dir) = temporary();
    write(&dir, "package.json", "{}");
    write(&dir, "existing/empty.txt", "");
    for (path, command) in [("existing/package.json", "existing"), ("apps/web/package.json", "new")] {
        let before = discover(&dir, &dir);
        write(&dir, path, &format!(r#"{{"scripts":{{"{command}":"true"}}}}"#));
        assert!(before.changed(), "{path}");
        assert!(command_names(&discover(&dir, &dir)).contains(&command));
    }
}

#[test]
fn replacements_with_the_same_mtime_are_detected() {
    let (_temp, dir) = temporary();
    write(&dir, "Makefile", "one:\n");
    let old_time = fs::metadata(filepath::join(&[&dir, "Makefile"])).unwrap().modified().unwrap();
    let before = discover(&dir, &dir);
    write(&dir, "replacement", "two:\n");
    fs::File::options()
        .write(true)
        .open(filepath::join(&[&dir, "replacement"]))
        .unwrap()
        .set_modified(old_time)
        .unwrap();
    fs::rename(filepath::join(&[&dir, "replacement"]), filepath::join(&[&dir, "Makefile"])).unwrap();
    assert!(before.changed());
    assert_eq!(command_names(&discover(&dir, &dir)), ["two"]);
}

#[test]
fn parent_lockfiles_and_maven_wrappers_are_dependencies() {
    let (_temp, dir) = temporary();
    write(&dir, ".git/HEAD", "");
    write(&dir, "apps/web/package.json", r#"{"scripts":{"dev":"vite"}}"#);
    write(&dir, "apps/web/pom.xml", "<project/>");
    let project = filepath::join(&[&dir, "apps/web"]);
    let before = discover(&project, &dir);
    write(&dir, "pnpm-lock.yaml", "");
    write(&dir, "mvnw", "#!/bin/sh\n");
    assert!(before.changed());
    let after = discover(&project, &dir);
    assert_eq!(after.files[0].commands[0].run, "pnpm run dev");
    assert_eq!(after.files[1].commands[0].run, "../../mvnw clean install");
    fs::remove_file(filepath::join(&[&dir, "pnpm-lock.yaml"])).unwrap();
    fs::remove_file(filepath::join(&[&dir, "mvnw"])).unwrap();
    assert!(after.changed());
    let without = discover(&project, &dir);
    assert_eq!(without.files[0].commands[0].run, "npm run dev");
    assert_eq!(without.files[1].commands[0].run, "mvn clean install");
}

#[test]
fn maven_modules_outside_the_fallback_scan_are_dependencies() {
    let (_temp, dir) = temporary();
    write(&dir, "pom.xml", "<project><modules><module>.modules/core</module></modules></project>");
    write(&dir, ".modules/core/pom.xml", "<project/>");
    let before = discover(&dir, &dir);
    edit(&dir, ".modules/core/pom.xml", "<project><profiles><profile><id>coverage</id></profile></profiles></project>");
    assert!(before.changed());
    assert!(command_names(&discover(&dir, &dir)).contains(&"clean install -Pcoverage"));
}

#[test]
fn unsupported_and_missing_includes_explain_partial_results() {
    let (_temp, dir) = temporary();
    write(
        &dir,
        "Taskfile.yml",
        "version: '3'\nincludes:\n  remote: https://example.invalid/Taskfile.yml\n  dynamic: '{{.FILE}}'\n  missing: gone.yml\ntasks:\n  build: echo ready\n",
    );
    write(&dir, "Makefile", "test:\n");
    let discovery = discover(&dir, &dir);
    assert_eq!(command_names(&discovery), ["build", "test"]);
    assert!(!discovery.complete());
    assert_eq!(discovery.diagnostics.len(), 3);
    assert!(discovery.files.iter().all(|file| file.err.is_none()));
}

#[test]
fn an_unreadable_folder_is_not_reported_as_a_complete_empty_project() {
    let (_temp, dir) = temporary();
    let missing = filepath::join(&[&dir, "missing"]);
    let before = discover(&missing, &missing);
    assert!(!before.complete());
    assert!(before.files.is_empty());
    assert!(!before.diagnostics.is_empty());
    write(&missing, "Makefile", "build:\n");
    assert!(before.changed());
}

#[test]
fn the_file_limit_is_strict_even_with_many_sources_in_one_folder() {
    let (_temp, dir) = temporary();
    write(&dir, "package.json", "{}");
    write(&dir, "Makefile", "build:\n");
    for index in 0..MAX_FILES {
        write(&dir, &format!("compose.{index:03}.yml"), "services: {}\n");
    }
    let discovery = discover(&dir, &dir);
    assert_eq!(discovery.files.len(), MAX_FILES);
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("file limit")));
    assert!(!discovery.complete());
    assert_eq!(command_names(&discovery), command_names(&discover(&dir, &dir)));
}

#[test]
fn include_reads_share_the_file_budget() {
    let (_temp, dir) = temporary();
    let mut entry = "version: '3'\ntasks:\n  own: echo own\nincludes:\n".to_string();
    for index in 0..MAX_FILES {
        entry += &format!("  n{index}: .tasks/{index}.yml\n");
        write(&dir, &format!(".tasks/{index}.yml"), "version: '3'\ntasks:\n  build: echo ready\n");
    }
    write(&dir, "Taskfile.yml", &entry);
    let discovery = discover(&dir, &dir);
    assert_eq!(discovery.files.len(), 1);
    assert_eq!(command_names(&discovery).len(), MAX_FILES);
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("file limit")));
}

#[test]
fn namespace_expansion_is_bounded_even_when_files_are_reused() {
    let (_temp, dir) = temporary();
    let mut entry = "version: '3'\nincludes:\n".to_string();
    for index in 0..1100 {
        entry += &format!("  n{index}: .tasks/shared.yml\n");
    }
    write(&dir, "Taskfile.yml", &entry);
    write(&dir, ".tasks/shared.yml", "version: '3'\ntasks:\n  build: echo ready\n");
    let discovery = discover(&dir, &dir);
    assert!(command_names(&discovery).len() < 1100);
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("expansion limit")));
}

#[test]
fn the_folder_budget_is_global_and_includes_the_root() {
    let (_temp, dir) = temporary();
    for parent in ["one", "two", "three"] {
        for index in 0..800 {
            fs::create_dir_all(filepath::join(&[&dir, parent, &format!("d{index:04}")])).unwrap();
        }
    }
    let mut scan = Scan::new(&dir);
    let mut ignores = crate::workspace::IgnoreRules::new(&dir, &mut scan);
    let folders = folders(&dir, true, None, false, &mut ignores, &mut scan);
    assert_eq!(folders.len(), MAX_FOLDERS);
    assert_eq!(scan.visited_folders.len(), MAX_FOLDERS);
    assert!(scan.dependencies.len() <= MAX_FOLDERS * 4);
    assert_eq!(scan.diagnostics.len(), 1);
    assert!(scan.diagnostics[0].message.contains("folder limit"));
    write(&dir, "package.json", r#"{"workspaces":["**"]}"#);
    let mut scan = Scan::new(&dir);
    let members = crate::workspace::declaration(&dir, &mut scan).unwrap();
    let mut ignores = crate::workspace::IgnoreRules::new(&dir, &mut scan);
    let mut selected = self::folders(&dir, true, Some(&members), true, &mut ignores, &mut scan);
    selected.extend(self::folders(&dir, true, Some(&members), false, &mut ignores, &mut scan));
    let paths: HashSet<_> = selected.iter().map(|folder| &folder.path).collect();
    assert_eq!(paths.len(), MAX_FOLDERS, "declared and incidental scans share one folder budget");
    assert_eq!(scan.visited_folders.len(), MAX_FOLDERS);
    write(&dir, "package.json", r#"{"workspaces":["**", "!one", "!two"]}"#);
    assert!(
        !discover(&dir, &dir).diagnostics.iter().any(|problem| problem.message.contains("folder limit")),
        "excluded subtrees do not consume the shared budget"
    );
}

#[test]
fn the_depth_limit_is_visible_even_when_usable_commands_were_found() {
    let (_temp, dir) = temporary();
    write(&dir, "Makefile", "build:\n");
    write(&dir, "a/b/c/d/Makefile", "deep:\n");
    let discovery = discover(&dir, &dir);
    assert_eq!(command_names(&discovery), ["build"]);
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("below depth 3")));
}

#[test]
fn maven_expansion_is_bounded_even_for_repeated_module_entries() {
    let (_temp, dir) = temporary();
    let modules = "<module>.modules/core</module>".repeat(1100);
    write(&dir, "pom.xml", &format!("<project><modules>{modules}</modules></project>"));
    write(&dir, ".modules/core/pom.xml", "<project/>");
    let discovery = discover(&dir, &dir);
    assert!(!discovery.complete());
    assert!(discovery.diagnostics.iter().any(|problem| problem.message.contains("expansion limit")));
    assert!(command_names(&discovery).contains(&"clean install"));
}
