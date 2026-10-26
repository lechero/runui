//! runs_test.go and output_test.go, ported.

use std::fs;
use std::time::{Duration, Instant};

use super::*;

/// SUPERVISE_ENV makes the test binary a supervisor, as runui _supervise
/// makes runui one: Go's TestMain is supervise_helper here, the one test a
/// supervisor started from this binary runs.
const SUPERVISE_ENV: &str = "RUNS_TEST_SUPERVISE";

#[test]
fn supervise_helper() {
    if std::env::var(SUPERVISE_ENV).as_deref() != Ok("1") {
        return;
    }
    set_stop_grace(Duration::from_millis(300));
    let folder = std::env::args().next_back().unwrap_or_default();
    std::process::exit(if supervise(&folder).is_ok() { 0 } else { 1 });
}

/// supervisor is how this test binary starts as a supervisor: the helper
/// test alone, the run's folder after it.
fn supervisor() -> Vec<String> {
    let exe = std::env::current_exe().unwrap().to_str().unwrap().to_string();
    vec![exe, "tests::supervise_helper".into(), "--exact".into(), "--test-threads=1".into(), "--quiet".into()]
}

fn tempdir() -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().to_str().unwrap().to_string();
    (t, p)
}

fn new_store(root: &str) -> Store {
    Store {
        root: filepath::join(&[root, "runs"]),
        exe: supervisor(),
        shell: vec!["/bin/sh".into(), "-c".into()],
        env: vec![format!("{SUPERVISE_ENV}=1")],
    }
}

/// wait_for waits for a run to get where ok says, and is it then.
#[track_caller]
fn wait_for(s: &Store, id: &str, what: &str, ok: impl Fn(&Run) -> bool) -> Run {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let r = s.get(id);
        if let Ok(r) = &r
            && ok(r)
        {
            return r.clone();
        }
        if Instant::now() > deadline {
            let out = r.as_ref().map(|r| fs::read_to_string(r.output()).unwrap_or_default()).unwrap_or_default();
            panic!("the run isn't {what}: {r:?}, output {out:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn output(r: &Run) -> String {
    fs::read_to_string(r.output()).unwrap()
}

fn printed(want: &str) -> impl Fn(&Run) -> bool + '_ {
    move |r| fs::read_to_string(r.output()).unwrap_or_default().contains(want)
}

fn start(s: &Store, dir: &str, name: &str, command: &str) -> Run {
    let (r, err) = s.start(dir, name, command);
    assert!(err.is_none(), "{err:?}");
    r
}

#[test]
fn a_run_is_stopped_from_another_store() {
    let (_t, root) = tempdir();
    let s = new_store(&root);
    let (_d, dir) = tempdir();
    let r = start(&s, &dir, "serve", r#"echo "in $(pwd)"; echo oops >&2; echo ready; sleep 30"#);
    let r = wait_for(&s, &r.id, "printing", printed("ready"));
    assert!(
        r.state() == State::Running && r.pid != 0 && r.supervisor != 0 && r.label() == "serve",
        "got {r:?} in state {:?}",
        r.state()
    );
    let got = output(&r);
    let real = fs::canonicalize(&dir).unwrap().to_str().unwrap().to_string();
    let in_dir = got.contains(&format!("in {dir}\n")) || got.contains(&format!("in {real}\n"));
    assert!(in_dir && got.contains("oops"), "output {got:?}: not in the folder, or without stderr");

    // The next runui: a store of its own over the same folder.
    let next = Store { root: s.root.clone(), ..Default::default() };
    let list = next.list().unwrap();
    assert!(list.len() == 1 && list[0].id == r.id && list[0].live(), "the next runui sees {list:?}");
    assert!(next.remove(&list[0]).is_err(), "a running run was removed");
    next.stop(&list[0]).unwrap();
    let r = wait_for(&s, &r.id, "stopped", |r| r.state() == State::Stopped);
    assert!(
        !r.signal.is_empty() && r.exit.is_some() && r.ended.is_some() && r.took(Utc::now()) > 0,
        "stopped, it's {r:?}"
    );
    assert!(!alive(r.pid), "the command, {}, is still there", r.pid);
    assert!(next.stop(&r).is_err(), "stopped a run that has ended");
    next.remove(&r).unwrap();
    let list = next.list().unwrap();
    assert!(list.is_empty(), "removed, it's still listed: {list:?}");
}

#[test]
fn a_run_ends_on_its_own() {
    let (_t, root) = tempdir();
    let s = new_store(&root);
    let (_d, dir) = tempdir();
    let r = start(&s, &dir, "", "echo done; exit 3");
    let r = wait_for(&s, &r.id, "ended", |r| r.ended.is_some());
    assert!(
        r.state() == State::Exited && r.exit == Some(3) && r.signal.is_empty() && r.label() == "echo done; exit 3",
        "got {r:?}"
    );
    assert_eq!(output(&r), "done\n");
}

#[test]
fn stopping_stops_the_whole_group() {
    let (_t, root) = tempdir();
    let s = new_store(&root);
    let (_d, dir) = tempdir();
    let r = start(&s, &dir, "", r#"sleep 30 & echo "child $!"; wait"#);
    let r = wait_for(&s, &r.id, "running its child", printed("child "));
    let pid: i32 = output(&r).trim().trim_start_matches("child ").parse().unwrap();
    s.stop(&r).unwrap();
    wait_for(&s, &r.id, "stopped", |r| r.state() == State::Stopped);
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(Instant::now() < deadline, "the background sleep, {pid}, outlived the stop");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_command_ignoring_term_is_killed() {
    let (_t, root) = tempdir();
    let s = new_store(&root);
    let (_d, dir) = tempdir();
    let r = start(&s, &dir, "", "trap '' TERM; echo ready; sleep 30");
    let r = wait_for(&s, &r.id, "ready", printed("ready"));
    s.stop(&r).unwrap();
    let r = wait_for(&s, &r.id, "killed", |r| r.state() == State::Stopped);
    assert_eq!(r.signal, "killed", "ended by {:?}, want killed", r.signal);
}

#[test]
fn stdin_stays_open() {
    let (_t, root) = tempdir();
    let s = new_store(&root);
    let (_d, dir) = tempdir();
    let r = start(&s, &dir, "", r#"echo reading; cat; echo "stdin closed""#);
    let r = wait_for(&s, &r.id, "reading", printed("reading"));
    std::thread::sleep(Duration::from_millis(300));
    let r = s.get(&r.id).unwrap();
    assert!(r.state() == State::Running && !output(&r).contains("closed"), "stdin closed: {r:?}, {:?}", output(&r));
    s.stop(&r).unwrap();
    wait_for(&s, &r.id, "stopped", |r| r.state() == State::Stopped);
}

#[test]
fn a_command_that_cant_start() {
    let (_t, root) = tempdir();
    let mut s = new_store(&root);
    s.shell = vec!["/no/such/shell".into(), "-c".into()];
    let (_d, dir) = tempdir();
    let r = start(&s, &dir, "", "true");
    let r = wait_for(&s, &r.id, "ended", |r| r.ended.is_some());
    assert!(
        r.state() == State::Exited && !r.error.is_empty() && r.exit == Some(127) && output(&r).contains("runui: "),
        "got {r:?}, output {:?}",
        output(&r)
    );
    assert_eq!(r.error, "fork/exec /no/such/shell: no such file or directory");
}

#[test]
fn starting_again_forgets_the_runs_that_ended() {
    let (_t, root) = tempdir();
    let s = new_store(&root);
    let (_d, dir) = tempdir();
    let first = start(&s, &dir, "", "echo one");
    wait_for(&s, &first.id, "ended", |r| r.ended.is_some());
    let other = start(&s, &dir, "", "echo other");
    wait_for(&s, &other.id, "ended", |r| r.ended.is_some());
    let second = start(&s, &dir, "", "echo one");
    wait_for(&s, &second.id, "ended", |r| r.ended.is_some());
    let list = s.list().unwrap();
    let ids: Vec<&str> = list.iter().map(|r| r.id.as_str()).collect();
    assert!(
        list.len() == 2 && list[0].id == second.id && list[1].id == other.id,
        "got {ids:?}, want {} then {}",
        second.id,
        other.id
    );
}

#[test]
fn a_run_whose_supervisor_went_is_lost() {
    let (_t, root) = tempdir();
    let s = new_store(&root);
    let mut r = Run {
        id: "20260101-000000-abcdef".into(),
        dir: "/p".into(),
        command: "npm run dev".into(),
        started: Utc::now(),
        supervisor: 1,
        pid: 2,
        ..Default::default()
    };
    r.folder = filepath::join(&[&s.root, &r.id]);
    fs::create_dir_all(&r.folder).unwrap();
    write_run(&r).unwrap();
    fs::write(filepath::join(&[&r.folder, LOCK_FILE]), "").unwrap();
    let got = s.get(&r.id).unwrap();
    assert!(got.state() == State::Lost && !got.live(), "state {:?}", got.state());
    assert!(s.stop(&got).is_err(), "stopped a lost run: it would signal a process that may not be its supervisor");
}

#[test]
fn supervise_wants_its_lock() {
    let (_t, root) = tempdir();
    let s = new_store(&root);
    let mut r = Run {
        id: "x".into(),
        command: "true".into(),
        shell: vec!["/bin/sh".into(), "-c".into()],
        started: Utc::now(),
        ..Default::default()
    };
    r.folder = filepath::join(&[&s.root, &r.id]);
    fs::create_dir_all(&r.folder).unwrap();
    write_run(&r).unwrap();
    fs::write(filepath::join(&[&r.folder, LOCK_FILE]), "").unwrap();
    assert!(supervise(&r.folder).is_err(), "supervised without the lock");
}

#[test]
fn start_without_a_supervisor() {
    let (_t, root) = tempdir();
    let mut s = Store { root: root.clone(), ..Default::default() };
    let (_, err) = s.start("/p", "", "true");
    assert!(err.is_some(), "started without a supervisor");
    s.exe = vec!["/no/such/runui".into()];
    let (_d, dir) = tempdir();
    let (r, err) = s.start(&dir, "", "true");
    assert!(err.is_some(), "started a supervisor that isn't there");
    let got = s.get(&r.id).unwrap();
    assert!(got.state() == State::Exited && !got.error.is_empty(), "got {got:?}, {err:?}");
    assert_eq!(got.error, "fork/exec /no/such/runui: no such file or directory");
}

// the environment is the process's: the tests that set it take turns
static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
#[allow(unsafe_code)]
fn defaults() {
    let _turn = ENV.lock().unwrap();
    let saved: Vec<(&str, Option<std::ffi::OsString>)> =
        ["XDG_STATE_HOME", "HOME", "PATH"].into_iter().map(|k| (k, std::env::var_os(k))).collect();
    let (_t, empty) = tempdir();
    // SAFETY: the tests that read or set the environment take turns, and
    // this one puts it back.
    unsafe { std::env::set_var("XDG_STATE_HOME", "/x/state") };
    assert_eq!(default_root().unwrap(), "/x/state/runui/runs", "with XDG_STATE_HOME");
    unsafe {
        std::env::set_var("XDG_STATE_HOME", "");
        std::env::set_var("HOME", "/home/me");
    }
    assert_eq!(default_root().unwrap(), "/home/me/.local/state/runui/runs", "without");
    unsafe { std::env::set_var("PATH", &empty) };
    assert_eq!(default_shell().join(" "), "/bin/sh -c", "without mise");
    for (k, v) in saved {
        unsafe {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}

#[test]
fn the_file_is_the_go_builds() {
    let (_t, root) = tempdir();
    let started = chrono::DateTime::parse_from_rfc3339("2026-10-25T19:24:00.5+02:00").unwrap().with_timezone(&Utc);
    let mut r = Run {
        id: "20261025-192400-0a0b0c".into(),
        dir: "/p".into(),
        command: "a && b".into(),
        shell: vec!["/bin/sh".into(), "-c".into()],
        started,
        supervisor: 10,
        pid: 11,
        ended: Some(started),
        exit: Some(0),
        ..Default::default()
    };
    r.folder = root.clone();
    write_run(&r).unwrap();
    let at = runui_core::golang::format_time(&started);
    assert_eq!(
        fs::read_to_string(filepath::join(&[&root, RUN_FILE])).unwrap(),
        format!(
            "{{\n  \"id\": \"20261025-192400-0a0b0c\",\n  \"dir\": \"/p\",\n  \"command\": \"a \\u0026\\u0026 b\",\n  \"shell\": [\n    \"/bin/sh\",\n    \"-c\"\n  ],\n  \"started\": \"{at}\",\n  \"supervisor\": 10,\n  \"pid\": 11,\n  \"ended\": \"{at}\",\n  \"exit\": 0\n}}\n"
        )
    );
    let back = read_run(&root).unwrap();
    assert_eq!(back.started, started);
    assert_eq!(back.exit, Some(0));
}

// output_test.go

#[test]
fn clean_lines() {
    for (input, want) in [
        ("plain", "plain"),
        ("crlf\r", "crlf"),
        ("10%\r50%\r100% done", "100% done"),
        ("\x1b[32mgreen\x1b[0m", "\x1b[32mgreen\x1b[0m"),
        ("\x1b[2K\x1b[1Gerased", "erased"),
        ("\x1b[2J\x1b[3J\x1b[Hcleared", "cleared"),
        ("\x1b]8;;https://x.dev\x07link\x1b]8;;\x07", "link"),
        ("\x1b]0;title\x1b\\after", "after"),
        ("\x1b(Bcharset", "charset"),
        ("a\tb", "a       b"),
        ("héllo\tx", "héllo   x"),
        ("bell\x07 and \x08backspace", "bell and backspace"),
        ("cut \x1b[", "cut "),
    ] {
        assert_eq!(clean(input), want, "clean({input:?})");
    }
}

fn append_to(path: &str, text: &str) {
    use std::io::Write;
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

#[test]
fn output_reads_as_its_printed() {
    let (_t, dir) = tempdir();
    let path = filepath::join(&[&dir, "output.log"]);
    append_to(&path, "one\ntw");
    let mut o = Output::new(&path, 3);
    assert!(o.read().unwrap(), "first read");
    assert_eq!(o.lines(), ["one", "tw"]);
    assert_eq!(o.range(1, 2), ["tw"], "the line still being printed, in a range");
    assert!(!o.read().unwrap(), "nothing new, but more");
    append_to(&path, "o\nthree\nfour\n");
    assert!(o.read().unwrap(), "new lines, but nothing");
    assert_eq!(o.lines(), ["two", "three", "four"], "the last three");
    assert_eq!(o.range(1, 9), ["three", "four"], "a range of them");
    fs::write(&path, "again\n").unwrap();
    o.read().unwrap();
    assert_eq!(o.lines(), ["again"], "a file started over");
}

#[test]
fn output_reads_the_end_of_a_long_one() {
    let (_t, dir) = tempdir();
    let path = filepath::join(&[&dir, "output.log"]);
    let line = format!("{}\n", "x".repeat(99));
    append_to(&path, &(line.repeat(2 * TAIL / line.len()) + "last\n"));
    let mut o = Output::new(&path, 100000);
    o.read().unwrap();
    let got = o.lines();
    assert!(
        got.len() <= TAIL / line.len() + 1 && got[got.len() - 1] == "last",
        "{} lines, the last {:?}",
        got.len(),
        got.last()
    );
    for l in &got[..got.len() - 1] {
        assert_eq!(l, line.trim_end_matches('\n'), "a line cut short");
    }
}

const TAIL: usize = 512 << 10;

#[test]
fn output_of_a_run_not_there_yet() {
    let (_t, dir) = tempdir();
    let mut o = Output::new(&filepath::join(&[&dir, "output.log"]), 10);
    assert!(o.read().is_err(), "no file, no error");
    assert!(o.lines().is_empty(), "lines from nowhere");
}

#[test]
fn pending_errors_exclude_live_stopped_successful_and_viewed_runs() {
    let (_t, root) = tempdir();
    let store = new_store(&root);
    let ended = Some(Utc::now());
    for (name, run, expected) in [
        ("starting", Run { live: true, ..Default::default() }, false),
        ("running", Run { live: true, pid: 123, ..Default::default() }, false),
        ("success", Run { ended, exit: Some(0), ..Default::default() }, false),
        ("failure", Run { ended, exit: Some(9), ..Default::default() }, true),
        ("unknown_exit", Run { ended, ..Default::default() }, true),
        ("lost", Run::default(), true),
        ("stopped", Run { ended, exit: Some(1), stopped: true, ..Default::default() }, false),
    ] {
        let run = run.with_folder(&filepath::join(&[&store.root, name]));
        assert_eq!(store.pending_error(&run), expected, "{name}");
        fs::create_dir_all(&run.folder).unwrap();
        fs::write(filepath::join(&[&run.folder, SEEN_FILE]), "").unwrap();
        assert!(!store.pending_error(&run), "a reviewed {name} was still counted");
    }
}
