use super::*;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Barrier};

fn run(id: &str, directory: &str, started: DateTime<Utc>) -> Run {
    serde_json::from_value(json!({"id":id,"name":"test","dir":directory,"command":"echo test","shell":["/bin/sh","-c"],"started":started.to_rfc3339(),"ended":started.to_rfc3339(),"exit":0})).unwrap()
}

#[test]
fn a_launch_is_counted_once_and_review_or_reconciliation_does_not_count_again() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::for_runs(temp.path().to_str().unwrap());
    let run = run("one", temp.path().to_str().unwrap(), Utc::now());
    store.record(&run, None).unwrap();
    store.record(&run, None).unwrap();
    store.reconcile(std::slice::from_ref(&run)).unwrap();
    let data = store.load().unwrap();
    assert_eq!(data.recent.len(), 1);
    assert_eq!(data.counts.values().next().unwrap().launches, 1);
    assert!(!temp.path().join("status.seen").exists());
}

#[test]
fn concurrent_writers_preserve_every_launch_and_readers_never_see_partial_json() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::for_runs(temp.path().to_str().unwrap());
    let barrier = Arc::new(Barrier::new(17));
    let mut threads = Vec::new();
    for index in 0..16 {
        let (store, barrier) = (store.clone(), barrier.clone());
        let directory = temp.path().to_str().unwrap().to_string();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            store.record(&run(&index.to_string(), &directory, Utc::now()), None).unwrap();
        }));
    }
    barrier.wait();
    for _ in 0..20 {
        assert!(store.load().is_ok());
        std::thread::yield_now();
    }
    for thread in threads {
        thread.join().unwrap();
    }
    let data = store.load().unwrap();
    assert_eq!(data.recent.len(), 16);
    assert_eq!(data.counts.values().next().unwrap().launches, 16);
    assert_eq!(fs::metadata(store.file()).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn output_pruning_does_not_prune_usage_and_detailed_retention_keeps_aggregates() {
    let now = Utc::now();
    let mut data = Data::default();
    for index in 0..1005 {
        data.record(&run(&index.to_string(), "/tmp/project", now - TimeDelta::seconds(index)), None, now).unwrap();
    }
    assert_eq!(data.recent.len(), RECENT_LIMIT);
    assert_eq!(data.counts.values().next().unwrap().launches, 1005);
    data.record(&run("old", "/tmp/project", now - TimeDelta::days(181)), None, now).unwrap();
    assert_eq!(data.counts.values().next().unwrap().launches, 1005);
    data.record(&run("1004", "/tmp/project", now - TimeDelta::seconds(1004)), None, now).unwrap();
    assert_eq!(
        data.counts.values().next().unwrap().launches,
        1005,
        "a pruned detail must not permit a duplicate launch"
    );
}

#[test]
fn corrupt_history_is_preserved_and_a_future_version_is_not_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::for_runs(temp.path().to_str().unwrap());
    fs::write(store.file(), b"{ interrupted").unwrap();
    assert!(store.load().is_err());
    store.record(&run("one", temp.path().to_str().unwrap(), Utc::now()), None).unwrap();
    let recovered = store.load().unwrap();
    assert_eq!(fs::read(recovered.recovered_from.unwrap()).unwrap(), b"{ interrupted");
    let future = b"{\"version\":2}";
    fs::write(store.file(), future).unwrap();
    assert!(store.record(&run("two", "/tmp", Utc::now()), None).is_err());
    assert_eq!(fs::read(store.file()).unwrap(), future);
}

#[test]
fn identical_text_in_different_directories_or_shells_keeps_distinct_usage() {
    let mut data = Data::default();
    let now = Utc::now();
    let first = run("one", "/tmp/one", now);
    let second = run("two", "/tmp/two", now);
    let mut third = run("three", "/tmp/one", now);
    third.shell = vec!["fish".into(), "-c".into()];
    for run in [&first, &second, &third] {
        data.record(run, None, now).unwrap();
    }
    assert_eq!(data.counts.keys().filter(|key| key.starts_with("invocation:")).count(), 3);
    assert!(data.counts.values().all(|count| count.launches == 1));
}

#[test]
fn raw_and_identified_launches_combine_without_double_counting_enrichment() {
    let temp = tempfile::tempdir().unwrap();
    let directory = canonical(temp.path().to_str().unwrap());
    let directory = directory.as_str();
    let store = Store::for_runs(directory);
    let now = Utc::now();
    let first = run("one", directory, now);
    let prepared = crate::Prepared {
        id: "cmd-v1-example".into(),
        fingerprint: "def-v1-example".into(),
        name: "test".into(),
        checkout: directory.into(),
        directory: directory.into(),
        command: first.command.clone(),
        shell: first.shell.clone(),
        inputs: crate::Inputs::default(),
        source: "make".into(),
    };
    store.record(&first, None).unwrap();
    store.record(&first, Some(&prepared)).unwrap();
    store.record(&first, Some(&prepared)).unwrap();
    store.record(&run("two", directory, now), None).unwrap();
    let data = store.load().unwrap();
    assert_eq!(data.counts["definition:cmd-v1-example"].launches, 1);
    assert_eq!(
        data.counts[&format!("unattributed:{}", invocation_key(directory, &first.command, &first.shell))].launches,
        1
    );
    assert_eq!(data.counts[&invocation_key(directory, &first.command, &first.shell)].launches, 2);
}

#[test]
fn oversized_history_is_not_loaded_or_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::for_runs(temp.path().to_str().unwrap());
    fs::File::create(store.file()).unwrap().set_len(MAX_BYTES + 1).unwrap();
    assert!(store.load().is_err());
    assert!(store.record(&run("one", "/tmp", Utc::now()), None).is_err());
    assert_eq!(fs::metadata(store.file()).unwrap().len(), MAX_BYTES + 1);
}

#[test]
fn a_missed_launch_record_is_recovered_from_run_metadata_without_counting_views() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::for_runs(temp.path().to_str().unwrap());
    let started = Utc::now() - TimeDelta::days(1);
    let completed = run("recovered", temp.path().to_str().unwrap(), started);
    store.reconcile(std::slice::from_ref(&completed)).unwrap();
    store.reconcile(std::slice::from_ref(&completed)).unwrap();
    let data = store.load().unwrap();
    assert_eq!(data.recent.len(), 1);
    assert_eq!(data.recent[0].started, started);
    assert!(data.counts.values().all(|count| count.launches == 1 && count.last_used == started));
    let stale: Run = serde_json::from_value(
        json!({"id":"recovered","dir":completed.dir,"command":completed.command,"started":started.to_rfc3339()}),
    )
    .unwrap();
    store.record(&stale, None).unwrap();
    assert_eq!(store.load().unwrap().recent[0].state.as_deref(), Some("completed"));
}

#[test]
fn observing_runs_applies_retention_without_fabricating_old_launches() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::for_runs(temp.path().to_str().unwrap());
    let now = Utc::now();
    let old = run("old", temp.path().to_str().unwrap(), now - TimeDelta::days(181));
    let mut data = Data::default();
    data.record(&old, None, now - TimeDelta::days(181)).unwrap();
    fs::write(store.file(), serde_json::to_vec(&data).unwrap()).unwrap();
    store.reconcile(std::slice::from_ref(&old)).unwrap();
    assert!(store.load().unwrap().recent.is_empty());
    assert!(store.load().unwrap().counts.values().all(|count| count.launches == 1));
}
