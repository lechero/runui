#!/usr/bin/env python3
"""Print discovery observations from disposable fixtures; never run their commands.

This is a research probe, not a test asserting that today's gaps must remain.
Requires the Rust toolchain and cached runui-project dependencies (Cargo offline).
Set CARGO_TARGET_DIR to reuse a build cache; otherwise all build output is temporary.
"""

import json
import os
from pathlib import Path
import subprocess
import tempfile

RUST = r'''
use std::{fs, path::Path, time::{Duration, UNIX_EPOCH}};
use runui_project::{changed, load, root, File};

fn put(base: &Path, name: &str, data: &str) {
    let path = base.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}
fn repository(base: &Path, name: &str) -> std::path::PathBuf {
    let path = base.join(name);
    fs::create_dir_all(path.join(".git")).unwrap();
    path
}
fn scan(path: &Path) -> Vec<File> {
    load(path.to_str().unwrap(), path.to_str().unwrap())
}
fn names(files: &[File]) -> Vec<String> {
    assert!(files.iter().all(|f| f.err.is_none()), "fixture parsing failed");
    files.iter().flat_map(|f| f.commands.iter().map(|c| c.name.clone())).collect()
}
fn main() {
    let base = std::env::args().nth(1).unwrap();
    let base = Path::new(&base);
    for (label, manifest, content) in [
        ("cargo", "Cargo.toml", "[package]\nname='demo'\nversion='0.1.0'\n"),
        ("composer", "composer.json", r#"{"scripts":{"test":"php test.php"}}"#),
    ] {
        let path = repository(base, label);
        put(&path, manifest, content);
        fs::create_dir_all(path.join("src/deep")).unwrap();
        let selected = root(path.join("src/deep").to_str().unwrap(), base.to_str().unwrap());
        println!(r#"{{"probe":"{label}","selects_manifest_root":{},"commands":{:?}}}"#,
            Path::new(&selected) == path, names(&scan(&path)));
    }
    let path = repository(base, "workspace");
    put(&path, "package.json", r#"{"scripts":{"root":"echo root"}}"#);
    put(&path, "pnpm-workspace.yaml", "packages:\n  - 'packages/**'\n");
    put(&path, "packages/a/b/c/package.json", r#"{"scripts":{"deep":"echo deep"}}"#);
    println!(r#"{{"probe":"deep_member","commands":{:?}}}"#, names(&scan(&path)));

    let path = repository(base, "includes");
    put(&path, "Taskfile.yml", "version: '3'\nincludes:\n  one: .tasks/shared.yml\n  two: .tasks/shared.yml\n");
    put(&path, ".tasks/shared.yml", "version: '3'\ntasks:\n  build:\n    desc: before\n    cmds: ['echo build']\n");
    let files = scan(&path);
    println!(r#"{{"probe":"include_namespaces","commands":{:?}}}"#, names(&files));
    let before = changed(path.to_str().unwrap(), &files);
    put(&path, ".tasks/shared.yml", "version: '3'\ntasks:\n  build:\n    desc: after\n    cmds: ['echo build']\n");
    let times = fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
    fs::File::options().write(true).open(path.join(".tasks/shared.yml")).unwrap().set_times(times).unwrap();
    let after = changed(path.to_str().unwrap(), &files);
    let reloaded = scan(&path).iter().any(|f| f.commands.iter().any(|c| c.desc == "after"));
    println!(r#"{{"probe":"include_refresh","invalidated":{},"reload_reads_edit":{reloaded}}}"#, before != after);

    let path = repository(base, "newfile");
    put(&path, "package.json", r#"{"scripts":{"root":"echo root"}}"#);
    let files = scan(&path);
    let before = changed(path.to_str().unwrap(), &files);
    put(&path, "apps/web/package.json", r#"{"scripts":{"new":"echo new"}}"#);
    println!(r#"{{"probe":"new_file_refresh","invalidated":{},"reloaded_commands":{:?}}}"#,
        before != changed(path.to_str().unwrap(), &files), names(&scan(&path)));

    let path = repository(base, "ignored");
    put(&path, "package.json", "{}");
    put(&path, ".gitignore", "scratch/\n");
    put(&path, "scratch/package.json", r#"{"scripts":{"ignored":"echo ignored"}}"#);
    println!(r#"{{"probe":"ignored_package","commands":{:?}}}"#, names(&scan(&path)));
}
'''


def main():
    repo = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="runui-discovery-probe-") as folder:
        work = Path(folder)
        (work / "src").mkdir()
        dependency = json.dumps(str(repo / "crates/runui-project"))
        (work / "Cargo.toml").write_text(
            '[package]\nname="runui-discovery-probe"\nversion="0.0.0"\nedition="2024"\n'
            f'[dependencies]\nrunui-project={{path={dependency}}}\n', encoding="utf-8"
        )
        (work / "src/main.rs").write_text(RUST, encoding="utf-8")
        env = os.environ.copy()
        env.setdefault("CARGO_TARGET_DIR", str(work / "target"))
        subprocess.run(
            ["cargo", "run", "--quiet", "--offline", "--manifest-path", str(work / "Cargo.toml"), "--", str(work / "fixtures")],
            env=env, check=True,
        )


if __name__ == "__main__":
    main()
