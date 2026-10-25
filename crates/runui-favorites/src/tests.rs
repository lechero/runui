//! favorites_test.go, ported.

use std::fs;
use std::sync::Mutex;

use super::*;

fn open(file: &str) -> Store {
    Store::open(file).unwrap()
}

fn commands(favs: &[Favorite]) -> Vec<String> {
    favs.iter().map(|f| f.command.clone()).collect()
}

fn fav(command: &str) -> Favorite {
    Favorite { command: command.to_string(), ..Default::default() }
}

fn tempfile(name: &str) -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().unwrap();
    let p = filepath::join(&[t.path().to_str().unwrap(), name]);
    (t, p)
}

#[test]
fn a_folder_keeps_its_favorites() {
    let (_t, file) = tempfile("runui/favorites.json");
    let mut s = open(&file);
    assert!(s.for_dir("/p/app").is_empty(), "a new store has {:?}", s.for_dir("/p/app"));
    s.add(
        "/p/app",
        Favorite { name: "start".into(), command: "npm run start".into(), source: "npm".into(), ..Default::default() },
    )
    .unwrap();
    s.add("/p/app/", fav("  docker compose up  ")).unwrap();
    s.add("/p/other", fav("make")).unwrap();

    let again = open(&file); // as the next runui reads it
    let got = again.for_dir("/p/app");
    assert_eq!(commands(&got), ["npm run start", "docker compose up"]);
    assert!(
        got[0].label() == "start" && !got[0].own() && got[1].label() == "docker compose up" && got[1].own(),
        "names or sources lost: {got:?}"
    );
    assert!(got[0].added.is_some(), "no time added");
    assert!(
        again.has("/p/app", &fav("npm run start")) && !again.has("/p/other", &fav("npm run start")),
        "has looks at the wrong folder"
    );
    assert_eq!(commands(&again.for_dir("/p/other")), ["make"]);
}

#[test]
fn adding_one_there_already_renames_it() {
    let (_t, file) = tempfile("favorites.json");
    let mut s = open(&file);
    s.add("/p", fav("make test")).unwrap();
    s.add("/p", fav("make lint")).unwrap();
    s.add("/p", Favorite { name: "tests".into(), command: "make test".into(), ..Default::default() }).unwrap();
    let got = s.for_dir("/p");
    assert!(got.len() == 2 && got[0].name == "tests" && got[0].command == "make test", "got {got:?}");
    assert!(
        s.add("/p", Favorite { name: "nothing".into(), command: "   ".into(), ..Default::default() }).is_err(),
        "a favorite without a command was added"
    );
}

#[test]
fn toggle() {
    let (_t, file) = tempfile("favorites.json");
    let mut s = open(&file);
    let f = Favorite { name: "dev".into(), command: "pnpm run dev".into(), source: "npm".into(), ..Default::default() };
    let added = s.toggle("/p", f.clone()).unwrap();
    assert!(added && s.has("/p", &fav(&f.command)), "starring: {added}");
    let added = s.toggle("/p", f.clone()).unwrap();
    assert!(!added && !s.has("/p", &fav(&f.command)), "unstarring: {added}");
}

#[test]
fn update() {
    let (_t, file) = tempfile("favorites.json");
    let mut s = open(&file);
    s.add(
        "/p",
        Favorite { name: "dev".into(), command: "pnpm run dev".into(), source: "npm".into(), ..Default::default() },
    )
    .unwrap();
    s.add("/p", fav("make")).unwrap();
    let added = s.for_dir("/p")[0].added;

    s.update(
        "/p",
        &fav("pnpm run dev"),
        Favorite { name: "the server".into(), command: "pnpm run dev".into(), ..Default::default() },
    )
    .unwrap();
    let got = &s.for_dir("/p")[0];
    assert!(got.name == "the server" && got.source == "npm" && got.added == added, "renamed, it's {got:?}");
    s.update(
        "/p",
        &fav("pnpm run dev"),
        Favorite { name: "the server".into(), command: "pnpm run dev --open".into(), ..Default::default() },
    )
    .unwrap();
    let got = &s.for_dir("/p")[0];
    assert!(got.command == "pnpm run dev --open" && got.own(), "with its command edited, it's {got:?}");
    assert!(s.update("/p", &fav("pnpm run dev --open"), fav("make")).is_err(), "two favorites with one command");
    assert!(s.update("/p", &fav("gone"), fav("x")).is_err(), "updated one that isn't there");
}

#[test]
fn remove_and_move() {
    let (_t, file) = tempfile("favorites.json");
    let mut s = open(&file);
    for c in ["a", "b", "c"] {
        s.add("/p", fav(c)).unwrap();
    }
    s.move_by("/p", &fav("c"), -1).unwrap();
    s.move_by("/p", &fav("a"), -1).unwrap(); // already first: stays
    assert_eq!(commands(&s.for_dir("/p")), ["a", "c", "b"], "moved");
    s.remove("/p", &fav("c")).unwrap();
    s.remove("/p", &fav("a")).unwrap();
    s.remove("/p", &fav("b")).unwrap();
    let data = fs::read_to_string(&file).unwrap();
    assert_eq!(data, "{\n  \"folders\": {}\n}\n", "a folder without favorites is kept");
}

#[test]
fn two_runuis_dont_undo_each_other() {
    let (_t, file) = tempfile("favorites.json");
    let (mut one, mut two) = (open(&file), open(&file));
    one.add("/p", fav("make")).unwrap();
    two.add("/p", fav("task")).unwrap(); // two read the file before one's change
    assert_eq!(commands(&open(&file).for_dir("/p")), ["make", "task"]);
    one.reload().unwrap();
    assert!(one.has("/p", &fav("task")), "reload didn't read the other's change");
}

#[test]
fn a_broken_file_is_an_error() {
    let (_t, file) = tempfile("favorites.json");
    fs::write(&file, "{not json").unwrap();
    let err = Store::open(&file).unwrap_err();
    assert_eq!(err.to_string(), format!("{file}: invalid character 'n' looking for beginning of object key string"));
}

#[test]
fn the_same_command_in_two_folders() {
    let (_t, file) = tempfile("favorites.json");
    let mut s = open(&file);
    let web = Favorite {
        name: "dev".into(),
        command: "pnpm run dev".into(),
        source: "npm".into(),
        dir: "apps/web".into(),
        ..Default::default()
    };
    let docs = Favorite { dir: "apps/docs".into(), ..web.clone() };
    s.add("/repo", web.clone()).unwrap();
    s.add("/repo", docs.clone()).unwrap();
    let got = s.for_dir("/repo");
    assert!(got.len() == 2 && got[0].dir == "apps/web" && got[1].dir == "apps/docs", "got {got:?}");
    assert!(!s.has("/repo", &fav("pnpm run dev")), "the project's own folder has it too");
    assert!(
        s.has("/repo", &Favorite { command: "pnpm run dev".into(), dir: "./apps/web/".into(), ..Default::default() }),
        "./apps/web/ isn't apps/web"
    );
    s.update(
        "/repo",
        &docs,
        Favorite { name: "the docs".into(), command: "pnpm run dev".into(), ..Default::default() },
    )
    .unwrap();
    let got = &s.for_dir("/repo")[1];
    assert!(got.name == "the docs" && got.dir == "apps/docs" && got.source == "npm", "renamed, it's {got:?}");
    let added = s.toggle("/repo", web).unwrap();
    assert!(!added, "toggling web");
    let got = s.for_dir("/repo");
    assert!(got.len() == 1 && got[0].dir == "apps/docs", "toggled the wrong one: {got:?}");
}

// the environment is the process's: the tests that set it take turns
static ENV: Mutex<()> = Mutex::new(());

#[test]
#[allow(unsafe_code)]
fn default_file_is_under_the_config_folder() {
    let _turn = ENV.lock().unwrap();
    let (config, home) = (std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"));
    // SAFETY: the tests that read or set the environment take turns.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", "/x/config") };
    assert_eq!(default_file().unwrap(), "/x/config/runui/favorites.json", "with XDG_CONFIG_HOME");
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", "");
        std::env::set_var("HOME", "/home/me");
    }
    assert_eq!(default_file().unwrap(), "/home/me/.config/runui/favorites.json", "without");
    unsafe {
        match config {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        if let Some(v) = home {
            std::env::set_var("HOME", v);
        }
    }
}

#[test]
fn the_file_is_the_go_builds() {
    let (_t, file) = tempfile("favorites.json");
    let mut s = open(&file);
    let added = chrono::DateTime::parse_from_rfc3339("2026-10-25T19:24:00.5+02:00").unwrap().with_timezone(&Utc);
    s.add(
        "/p",
        Favorite {
            name: "dev".into(),
            command: "a <b> && c".into(),
            source: "npm".into(),
            dir: "apps/web".into(),
            added: Some(added),
        },
    )
    .unwrap();
    let data = fs::read_to_string(&file).unwrap();
    let want_added = runui_core::golang::format_time(&added);
    assert_eq!(
        data,
        format!(
            "{{\n  \"folders\": {{\n    \"/p\": [\n      {{\n        \"name\": \"dev\",\n        \"command\": \"a \\u003cb\\u003e \\u0026\\u0026 c\",\n        \"source\": \"npm\",\n        \"dir\": \"apps/web\",\n        \"added\": \"{want_added}\"\n      }}\n    ]\n  }}\n}}\n"
        )
    );
}

// what Go 1.27's build said of each, less the file's name before it
#[test]
fn the_file_reads_as_gos_decoder_reads_it() {
    let read = |body: &str| -> Result<Vec<(String, Vec<String>)>, String> {
        let (_t, file) = tempfile("favorites.json");
        fs::write(&file, body).unwrap();
        let s = Store::open(&file).map_err(|e| e.to_string()[file.len() + 2..].to_string())?;
        Ok(s.folders.iter().map(|(k, v)| (k.clone(), commands(v))).collect())
    };
    let err = |body: &str| read(body).unwrap_err();
    assert_eq!(err("[]"), "json: cannot unmarshal array into Go value of type favorites.fileData");
    assert_eq!(
        err(r#"{"folders": 5}"#),
        "json: cannot unmarshal number into Go struct field fileData.folders of type map[string][]favorites.Favorite"
    );
    assert_eq!(
        err(r#"{"folders": "x"}"#),
        "json: cannot unmarshal string into Go struct field fileData.folders of type map[string][]favorites.Favorite"
    );
    assert_eq!(
        err(r#"{"folders": true}"#),
        "json: cannot unmarshal bool into Go struct field fileData.folders of type map[string][]favorites.Favorite"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": 5}}"#),
        "json: cannot unmarshal number into Go struct field fileData.folders.~1p of type []favorites.Favorite"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": {}}}"#),
        "json: cannot unmarshal object into Go struct field fileData.folders.~1p of type []favorites.Favorite"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": [5]}}"#),
        "json: cannot unmarshal number into fileData.folders.~1p.0 of type favorites.Favorite"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": ["s"]}}"#),
        "json: cannot unmarshal string into fileData.folders.~1p.0 of type favorites.Favorite"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": [[1]]}}"#),
        "json: cannot unmarshal array into fileData.folders.~1p.0 of type favorites.Favorite"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": [{"Command": 5}]}}"#),
        "json: cannot unmarshal number into Go struct field fileData.folders.~1p.0.Command of type string"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": [{"command": [1]}]}}"#),
        "json: cannot unmarshal array into Go struct field fileData.folders.~1p.0.command of type string"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": [{"name": {}}]}}"#),
        "json: cannot unmarshal object into Go struct field fileData.folders.~1p.0.name of type string"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": [{"command": true}]}}"#),
        "json: cannot unmarshal bool into Go struct field fileData.folders.~1p.0.command of type string"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": [{"command": 1.5}]}}"#),
        "json: cannot unmarshal number into Go struct field fileData.folders.~1p.0.command of type string"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": [{"added": 5}]}}"#),
        "json: cannot unmarshal number into Go struct field fileData.folders.~1p.0.added of type time.Time"
    );
    assert_eq!(
        err(r#"{"folders": {"a~b/c": [{"command": 5}]}}"#),
        "json: cannot unmarshal number into Go struct field fileData.folders.a~0b~1c.0.command of type string"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": [{"command": "x"}, {"command": "y", "dir": 3}]}}"#),
        "json: cannot unmarshal number into Go struct field fileData.folders.~1p.1.dir of type string"
    );
    assert_eq!(
        err(r#"{"folders": {"/p": [{"command": "x", "added": "not a time"}]}}"#),
        r#"parsing time "not a time" as "2006-01-02T15:04:05Z07:00": cannot parse "not a time" as "2006""#
    );
    assert_eq!(err("{not json"), "invalid character 'n' looking for beginning of object key string");
    let ok = |body: &str| read(body).unwrap();
    let one =
        |dir: &str, cmds: &[&str]| vec![(dir.to_string(), cmds.iter().map(|c| c.to_string()).collect::<Vec<_>>())];
    assert_eq!(ok(r#"{"Folders": {"/p": [{"command": "x"}]}}"#), one("/p", &["x"]));
    assert_eq!(ok(r#"{"folders": {"/p": [{"COMMAND": "x", "command": null}]}}"#), one("/p", &["x"]));
    assert_eq!(ok(r#"{"folders": {"/p": [{"command": "a", "command": "b"}]}}"#), one("/p", &["b"]));
    assert_eq!(ok(r#"{"folders": {"/p": [null, {"command": "y"}]}}"#), one("/p", &["", "y"]));
    assert_eq!(ok(r#"{"folders": {"/p": null}}"#), one("/p", &[]));
    assert_eq!(ok(r#"{"folders": null}"#), vec![]);
    assert_eq!(ok("null"), vec![]);
    assert_eq!(
        ok(r#"{"folders": {"/p": [{"command": "x"}]}, "folders": {"/q": [{"command": "y"}]}}"#),
        vec![("/p".to_string(), vec!["x".to_string()]), ("/q".to_string(), vec!["y".to_string()])]
    );
}
