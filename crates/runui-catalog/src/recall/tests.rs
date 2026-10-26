use super::*;

fn fixture(dialect: Dialect, bytes: impl AsRef<[u8]>) -> (tempfile::TempDir, Source) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history");
    fs::write(&path, bytes).unwrap();
    (temp, Source { dialect, path: Some(path) })
}
fn read(source: &Source) -> Data {
    load(source, &AtomicBool::new(false))
}

#[test]
fn fish_is_a_native_format_not_yaml_and_never_infers_cwd_or_frequency() {
    let bytes=b"- cmd: echo literal: #yaml\\\\n\n  when: 1700000001\n  paths:\n    - /not/the/cwd\n  added_when: 1700000000\n- cmd: begin\\n echo coffee ";
    let mut bytes = bytes.to_vec();
    bytes.extend("☕\\nend\n  when: 1700000002\n- cmd: echo literal: #yaml\\\\n\n  when: 1700000003\n".as_bytes());
    let (_temp, source) = fixture(Dialect::Fish, &bytes);
    let data = read(&source);
    assert_eq!(data.entries.len(), 2);
    assert_eq!(data.entries[0].command, "echo literal: #yaml\\n");
    assert_eq!(data.entries[0].timestamp.unwrap().timestamp(), 1700000003);
    assert_eq!(data.entries[1].command, "begin\n echo coffee ☕\nend");
    assert!(data.entries.iter().all(|entry| entry.directory.is_none() && entry.exit_code.is_none()));
    assert_eq!(fs::read(source.path.unwrap()).unwrap(), bytes);
}

#[test]
fn interrupted_invalid_bytes_unknown_escapes_and_nul_are_diagnosed_not_repaired() {
    let (_temp,source)=fixture(Dialect::Fish,b"- cmd: good\n  when: 12\n  paths:\n    - /unknown\n- cmd: bad\\qescape\n  when: 14\n- cmd: invalid\xff\n  when: 15\n- cmd: nul\0value\n  when: 16\n- cmd: unfinished\n  when: 1");
    let data = read(&source);
    assert_eq!(data.entries.len(), 1);
    assert_eq!(data.entries[0].command, "good");
    assert_eq!(data.entries[0].metadata_fields, ["paths"]);
    assert!(data.diagnostics.iter().any(|message| message.contains("interrupted")));
    assert!(data.diagnostics.iter().any(|message| message.contains("non-UTF-8")));
    assert!(data.diagnostics.iter().any(|message| message.contains("unsupported escapes")));
}

#[test]
fn zsh_extended_plain_metafied_and_multiline_records_round_trip() {
    let (_temp,source)=fixture(Dialect::Zsh,b": 1700000000:7;begin\\\n echo Unicode \xe2\x83\xb8\x83\xb5\\\nend\n: 1700000001:0;echo terminal\\ \n\\: literal prefix\n: bad-header\n");
    let data = read(&source);
    assert_eq!(data.entries.len(), 3);
    assert_eq!(data.entries[0].command, ": literal prefix");
    assert!(data.entries[0].timestamp.is_none());
    assert_eq!(data.entries[1].command, "echo terminal\\");
    assert_eq!(data.entries[2].command, "begin\n echo Unicode ☕\nend");
    assert_eq!(data.entries[2].duration_seconds, Some(7));
    assert!(data.diagnostics.iter().any(|message| message.contains("Malformed extended")));
}

#[test]
fn bash_requires_record_boundaries_and_retains_newlines_and_unknown_outcomes() {
    let (_temp, source) = fixture(
        Dialect::Bash,
        "ambiguous\nplain\n#1700000001\nprintf 'first\nsecond'\n#1700000002\necho '\\n'\n#1700000003\npartial",
    );
    let data = read(&source);
    assert_eq!(data.entries.len(), 2);
    assert_eq!(data.entries[0].command, "echo '\\n'");
    assert_eq!(data.entries[1].command, "printf 'first\nsecond'");
    assert!(data.diagnostics.iter().any(|message| message.contains("ambiguous")));
    assert!(data.entries.iter().all(|entry| entry.duration_seconds.is_none() && entry.exit_code.is_none()));
    let (_temp, plain) = fixture(Dialect::Bash, "one\ntwo\n");
    assert!(read(&plain).entries.is_empty());
}

#[test]
fn byte_and_candidate_limits_cancellation_rotation_and_disable_are_observable() {
    let (_temp, source) =
        fixture(Dialect::Fish, "- cmd: first\n  when: 1\n- cmd: second\n  when: 2\n- cmd: last\n  when: 3\n");
    let data = load_with_limit(&source, &AtomicBool::new(false), 40, 1);
    assert!(data.bytes_read <= 40);
    assert_eq!(data.entries.len(), 1);
    assert_eq!(data.entries[0].command, "last");
    assert!(!data.changed());
    let old = source.path.as_ref().unwrap();
    let new = old.with_extension("new");
    fs::write(&new, "- cmd: replacement\n  when: 4\n").unwrap();
    fs::rename(new, old).unwrap();
    assert!(data.changed());
    assert_eq!(read(&source).entries[0].command, "replacement");
    let data = load(&source, &AtomicBool::new(true));
    assert_eq!(data.bytes_read, 0);
    assert!(data.entries.is_empty());
    let disabled = read(&Source { dialect: Dialect::Fish, path: None });
    assert_eq!(disabled.bytes_read, 0);
    let (_temp, dir) = fixture(Dialect::Fish, "");
    let mut source = dir;
    source.path = source.path.map(|path| path.parent().unwrap().into());
    assert!(read(&source).diagnostics.iter().any(|message| message.contains("regular file")));
}

#[test]
fn search_is_literal_and_exact_replay_requires_current_record_cwd_and_matching_shell() {
    let (_temp, source) =
        fixture(Dialect::Zsh, ": 1700000001:0;echo source:cargo\n: 1700000002:0;touch NEVER $(printf inert)\n");
    let data = read(&source);
    let hits = data.search("source:cargo", 5, 0);
    assert_eq!(hits.len(), 1);
    let id = &hits[0].id;
    let dir = source.path.as_ref().unwrap().parent().unwrap().to_string_lossy();
    assert!(prepare(&source, id, "", "/bin/zsh", &AtomicBool::new(false)).is_err());
    assert!(prepare(&source, id, &dir, "/bin/sh", &AtomicBool::new(false)).is_err());
    fs::write(source.path.as_ref().unwrap(), "different\n").unwrap();
    // No process is started: missing shell and a removed record both fail.
    assert!(prepare(&source, id, &dir, "/does/not/exist/zsh", &AtomicBool::new(false)).is_err());
    assert!(!Path::new(dir.as_ref()).join("NEVER").exists());
}

#[test]
fn preparation_retains_native_text_and_rejects_a_removed_record_before_any_action() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, source) = fixture(Dialect::Fish, "- cmd: begin\\n echo 'literal' \\\\n\\nend\n  when: 1700000000\n");
    let program = temp.path().join("fish");
    fs::write(&program, "#!/bin/sh\nexit 91\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let data = read(&source);
    let entry = &data.entries[0];
    let prepared = prepare(
        &source,
        &entry.id,
        &temp.path().to_string_lossy(),
        &program.to_string_lossy(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(prepared.command, entry.command);
    assert_eq!(prepared.shell, [program.to_string_lossy().as_ref(), "-c"]);
    assert!(prepared.favorite_command().ends_with(&crate::quote_arg(&entry.command)));
    fs::write(source.path.as_ref().unwrap(), "- cmd: different\n  when: 1\n").unwrap();
    assert!(
        prepare(
            &source,
            &entry.id,
            &temp.path().to_string_lossy(),
            &program.to_string_lossy(),
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string()
        .contains("no longer available")
    );
}

#[test]
fn tiny_and_invalid_source_records_have_bounded_candidates_and_diagnostics() {
    for dialect in [Dialect::Fish, Dialect::Zsh, Dialect::Bash] {
        let bytes: String = (0..12_000)
            .map(|i| match dialect {
                Dialect::Fish => format!("- cmd: echo {i}\n  when: 1\n"),
                Dialect::Zsh => format!("echo {i}\n"),
                Dialect::Bash => format!("#1700000000\necho {i}\n"),
            })
            .collect();
        let (_temp, source) = fixture(dialect, bytes);
        let data = read(&source);
        assert_eq!(data.entries.len(), MAX_ENTRIES);
        assert_eq!(data.entries[0].command, "echo 11999");
        assert!(data.diagnostics.iter().any(|note| note.contains("10000 source records")));
    }
    let (_temp, source) = fixture(Dialect::Zsh, ": malformed\n".repeat(100_000));
    let data = read(&source);
    assert!(data.entries.is_empty());
    assert_eq!(data.diagnostics.len(), 1);
}
