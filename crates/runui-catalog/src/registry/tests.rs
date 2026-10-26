use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Barrier};

fn checkout(root: &Path, name: &str) -> PathBuf {
    let directory = root.join(name);
    fs::create_dir_all(directory.join(".git")).unwrap();
    fs::write(directory.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(directory.join("Makefile"), "test:\n\ttrue\n").unwrap();
    directory
}

#[test]
fn registrations_are_explicit_checkout_qualified_and_forgetting_keeps_files() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::new(home.path().join("config/projects-v1.json"));
    assert!(store.load().unwrap().projects.is_empty());
    assert!(!home.path().join("config").exists());
    let first = checkout(home.path(), "one/app");
    let other = checkout(home.path(), "two/app");
    fs::create_dir(first.join("src")).unwrap();
    let a = store.add(first.join("src").to_str().unwrap(), home.path().to_str().unwrap(), Some("App")).unwrap();
    let b = store.add(other.to_str().unwrap(), home.path().to_str().unwrap(), Some("App")).unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!(a.directory, first.canonicalize().unwrap().to_str().unwrap());
    let alias = home.path().join("alias");
    std::os::unix::fs::symlink(&first, &alias).unwrap();
    let again = store.add(alias.to_str().unwrap(), home.path().to_str().unwrap(), None).unwrap();
    assert_eq!(again, a);
    assert_eq!(store.load().unwrap().projects.len(), 2);
    assert!(store.forget(&a.id).unwrap());
    assert!(!store.forget(&a.id).unwrap());
    assert!(first.join("Makefile").exists());
    assert!(store.project(&b.id).is_ok());
    assert_eq!(fs::metadata(store.file()).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn simultaneous_registry_writers_keep_every_checkout_and_unknown_versions() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::new(home.path().join("projects-v1.json"));
    let barrier = Arc::new(Barrier::new(12));
    std::thread::scope(|scope| {
        for index in 0..12 {
            let directory = checkout(home.path(), &format!("app-{index}"));
            let (barrier, store) = (barrier.clone(), store.clone());
            scope.spawn(move || {
                barrier.wait();
                store.add(directory.to_str().unwrap(), "/", None).unwrap();
            });
        }
    });
    assert_eq!(store.load().unwrap().projects.len(), 12);
    for version in [serde_json::json!(2), serde_json::json!("future")] {
        let bytes = serde_json::json!({"version":version,"projects":[]}).to_string();
        fs::write(store.file(), &bytes).unwrap();
        assert!(store.load().is_err());
        assert!(store.add(home.path().to_str().unwrap(), "/", Some("test")).is_err());
        assert_eq!(fs::read_to_string(store.file()).unwrap(), bytes);
    }
}

#[test]
fn damaged_registry_is_backed_up_and_oversized_state_is_never_replaced() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::new(home.path().join("projects-v1.json"));
    fs::write(store.file(), b"{interrupted").unwrap();
    assert!(store.load().is_err());
    let directory = checkout(home.path(), "app");
    store.add(directory.to_str().unwrap(), "/", None).unwrap();
    let data = store.load().unwrap();
    assert_eq!(fs::read(data.recovered_from.unwrap()).unwrap(), b"{interrupted");
    fs::File::create(store.file()).unwrap().set_len(MAX_BYTES + 1).unwrap();
    assert!(store.add(directory.to_str().unwrap(), "/", None).is_err());
    assert_eq!(fs::metadata(store.file()).unwrap().len(), MAX_BYTES + 1);
}

#[test]
fn missing_and_retargeted_checkouts_are_not_reinterpreted_or_removed_implicitly() {
    let home = tempfile::tempdir().unwrap();
    let directory = checkout(home.path(), "app");
    let store = Store::new(home.path().join("projects-v1.json"));
    let registered = store.add(directory.to_str().unwrap(), "/", None).unwrap();
    fs::remove_dir_all(&directory).unwrap();
    assert!(store.project(&registered.id).unwrap().check_directory().is_err());
    let other = checkout(home.path(), "other");
    std::os::unix::fs::symlink(other, &directory).unwrap();
    assert!(store.project(&registered.id).unwrap().check_directory().is_err());
    assert_eq!(store.load().unwrap().projects.len(), 1);
    assert!(store.forget(directory.to_str().unwrap()).unwrap());
    assert!(!store.forget(&registered.id).unwrap());
}

#[test]
fn registry_rejects_invalid_names_identities_and_excess_projects() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::new(home.path().join("projects-v1.json"));
    let directory = checkout(home.path(), "app");
    assert!(store.add(directory.to_str().unwrap(), "/", Some("bad\nname")).is_err());
    for index in 0..MAX_PROJECTS {
        let directory = checkout(home.path(), &format!("app-{index}"));
        store.add(directory.to_str().unwrap(), "/", None).unwrap();
    }
    assert!(store.add(directory.to_str().unwrap(), "/", None).is_err());
    let mut data = store.load().unwrap();
    data.projects[0].id = "invented".into();
    fs::write(store.file(), serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(store.load().is_err());
}
