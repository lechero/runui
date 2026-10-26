use super::*;
use crate::{Context, Environment, Inputs, Query, Snapshot, registry, usage};
use std::sync::atomic::AtomicUsize;

fn project(root: &Path, name: &str) -> Project {
    let directory = root.join(name);
    fs::create_dir_all(directory.join(".git")).unwrap();
    fs::write(directory.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(directory.join("Makefile"), "test:\n\ttrue\n").unwrap();
    registry::Store::new(root.join("projects.json")).add(directory.to_str().unwrap(), "/", Some("same-name")).unwrap()
}

fn pause(dir: &str, stop: &str, cancel: Arc<AtomicBool>) -> Discovery {
    while Path::new(dir).join("pause").exists() && !cancel.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(5));
    }
    runui_project::discover_cancellable(dir, stop, cancel)
}

#[test]
fn a_slow_or_missing_checkout_does_not_hold_back_other_results() {
    let temp = tempfile::tempdir().unwrap();
    let projects: Vec<_> = (0..4).map(|index| project(temp.path(), &format!("app-{index}"))).collect();
    fs::write(Path::new(&projects[0].directory).join("pause"), "").unwrap();
    fs::remove_dir_all(&projects[1].directory).unwrap();
    let index = Index::default();
    index.request_with(&projects, pause);
    let view = index.wait(Duration::from_millis(800));
    assert!(view[0].pending);
    assert!(view[1].error.as_deref().unwrap().contains("unavailable"));
    assert!(view[2..].iter().all(|project| project.discovery.is_some() && !project.pending));
    fs::remove_file(Path::new(&projects[0].directory).join("pause")).unwrap();
    assert!(index.wait(Duration::from_secs(3)).iter().all(|project| !project.pending));
}

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static HIGH: AtomicUsize = AtomicUsize::new(0);
fn counted(dir: &str, stop: &str, cancel: Arc<AtomicBool>) -> Discovery {
    let active = ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;
    HIGH.fetch_max(active, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(20));
    let discovery = runui_project::discover_cancellable(dir, stop, cancel);
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
    discovery
}

#[test]
fn work_is_bounded_to_four_workers_and_obsolete_generations_do_not_restore_projects() {
    let temp = tempfile::tempdir().unwrap();
    let projects: Vec<_> = (0..12).map(|index| project(temp.path(), &format!("app-{index:02}"))).collect();
    let index = Index::default();
    index.request_with(&projects, counted);
    index.request_with(&projects[8..], counted);
    let view = index.wait(Duration::from_secs(5));
    assert_eq!(view.len(), 4);
    assert!(view.iter().all(|item| !item.pending && item.discovery.is_some()));
    assert!(HIGH.load(Ordering::SeqCst) <= WORKERS);
    assert!(index.request_with(&[], counted).is_empty());
    assert!(index.wait(Duration::from_millis(50)).is_empty());
}

#[test]
fn cached_definitions_are_reused_and_source_or_branch_changes_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let project = project(temp.path(), "app");
    let index = Index::default();
    index.request(std::slice::from_ref(&project));
    let before = index.wait(Duration::from_secs(3));
    index.invalidate();
    index.request(std::slice::from_ref(&project));
    let unchanged = index.wait(Duration::from_secs(3));
    assert!(Arc::ptr_eq(before[0].discovery.as_ref().unwrap(), unchanged[0].discovery.as_ref().unwrap()));
    fs::write(Path::new(&project.directory).join("Makefile"), "updated:\n\ttrue\n").unwrap();
    fs::write(Path::new(&project.directory).join(".git/HEAD"), "ref: refs/heads/feature\n").unwrap();
    index.invalidate();
    index.request(std::slice::from_ref(&project));
    let after = index.wait(Duration::from_secs(3));
    assert_eq!(after[0].branch.as_deref(), Some("feature"));
    assert_eq!(after[0].discovery.as_ref().unwrap().files[0].commands[0].name, "updated");
}

#[test]
fn registered_catalogs_qualify_duplicate_names_and_keep_scope_independent_ids() {
    let temp = tempfile::tempdir().unwrap();
    let a = project(temp.path(), "one/app");
    let b = project(temp.path(), "two/app");
    let index = Index::default();
    index.request(&[a.clone(), b]);
    let indexed = index.wait(Duration::from_secs(3));
    let favorites = runui_favorites::Store::open(&temp.path().join("favorites.json").to_string_lossy()).unwrap();
    let snapshot = Snapshot::registered(
        &indexed,
        &Environment::default(),
        &a.directory,
        &favorites,
        &[],
        &usage::Data::default(),
        false,
    );
    let hits = snapshot.search(&Query::parse("test").unwrap());
    assert_eq!(hits.len(), 2);
    assert_ne!(hits[0].entry.id, hits[1].entry.id);
    assert_eq!(hits[0].entry.registry_id.as_deref(), Some(a.id.as_str()));
    let local = Snapshot::build(
        &Context::new(&a.directory, &a.directory),
        indexed[0].discovery.as_ref().unwrap(),
        &[],
        &[],
        &usage::Data::default(),
    );
    assert_eq!(local.entries()[0].id, hits[0].entry.id);
    assert_eq!(
        snapshot.prepare(&hits[1].entry.id, None, &Inputs::default()).unwrap().directory,
        hits[1].entry.directory
    );
    assert!(snapshot.complete());
}

#[test]
fn worktree_gitfiles_and_detached_heads_are_labels_only() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("worktree");
    fs::create_dir(&directory).unwrap();
    fs::create_dir(temp.path().join("gitdir")).unwrap();
    fs::write(directory.join(".git"), "gitdir: ../gitdir\n").unwrap();
    fs::write(temp.path().join("gitdir/HEAD"), "ref: refs/heads/topic\n").unwrap();
    assert_eq!(branch(directory.to_str().unwrap()).as_deref(), Some("topic"));
    fs::write(temp.path().join("gitdir/HEAD"), "0123456789abcdef0123456789abcdef01234567\n").unwrap();
    assert_eq!(branch(directory.to_str().unwrap()).as_deref(), Some("detached 0123456789ab"));
}

#[test]
fn project_filter_precedes_pagination_and_registered_results_have_explicit_limits() {
    let temp = tempfile::tempdir().unwrap();
    let a = project(temp.path(), "a");
    let b = project(temp.path(), "b");
    let index = Index::default();
    index.request(&[a.clone(), b.clone()]);
    let indexed = index.wait(Duration::from_secs(3));
    let favorites = runui_favorites::Store::open(&temp.path().join("favorites.json").to_string_lossy()).unwrap();
    let mut snapshot = Snapshot::registered(
        &indexed,
        &Environment::default(),
        &a.directory,
        &favorites,
        &[],
        &usage::Data::default(),
        false,
    );
    let prototype = snapshot.entries[0].clone();
    for number in 0..300 {
        let mut entry = prototype.clone();
        entry.id = format!("synthetic-{number}");
        snapshot.entries.push(entry);
    }
    let mut query = Query::parse("test").unwrap();
    query.registry_id = Some(b.id);
    assert_eq!(snapshot.search(&query).len(), 1);
    let mut discovery = indexed[0].discovery.as_ref().unwrap().as_ref().clone();
    let prototype = discovery.files[0].commands[0].clone();
    discovery.files[0].commands = (0..10_001)
        .map(|number| {
            let mut command = prototype.clone();
            command.name = format!("test-{number}");
            command
        })
        .collect();
    let indexed =
        [Indexed { project: a, discovery: Some(Arc::new(discovery)), branch: None, error: None, pending: false }];
    let snapshot =
        Snapshot::registered(&indexed, &Environment::default(), "/", &favorites, &[], &usage::Data::default(), false);
    assert_eq!(snapshot.entries().len(), 10_000);
    assert!(!snapshot.complete());
    assert!(snapshot.diagnostics.iter().any(|item| item.kind == "limit"));
}
