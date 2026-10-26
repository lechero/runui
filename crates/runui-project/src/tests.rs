//! project_test.go and subfolders_test.go, ported.

use std::collections::BTreeMap;
use std::fs;
use std::time::{Duration, SystemTime};

use super::*;

/// write puts files in a folder, making the folders they're in.
fn write(dir: &str, files: &[(&str, &str)]) {
    for (name, body) in files {
        let path = filepath::join(&[dir, name]);
        fs::create_dir_all(filepath::dir(&path)).unwrap();
        fs::write(&path, body).unwrap();
    }
}

fn tempdir() -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().unwrap();
    // the path as the Go tests' t.TempDir() names it: not through /private
    let p = t.path().to_str().unwrap().to_string();
    (t, p)
}

/// brief is a command as name=run (desc), for comparing.
fn brief(cmds: &[Command]) -> Vec<String> {
    cmds.iter()
        .map(|c| {
            let mut s = format!("{}={}", c.name, c.run);
            if !c.desc.is_empty() {
                s += &format!(" ({})", c.desc);
            }
            s
        })
        .collect()
}

#[track_caller]
fn check(got: &[Command], want: &[&str]) {
    let b = brief(got);
    assert_eq!(b, want, "got\n  {}\nwant\n  {}", b.join("\n  "), want.join("\n  "));
}

#[test]
fn taskfile() {
    let (_t, dir) = tempdir();
    write(
        &dir,
        &[
            (
                "Taskfile.yml",
                r#"version: '3'
includes:
  docs: ./docs
  lib:
    taskfile: ./lib/Tasks.yml
    flatten: true
  secret:
    taskfile: ./secret
    internal: true
  gone:
    taskfile: ./not-there
    optional: true
  remote: https://example.com/Taskfile.yml
tasks:
  default:
    desc: List every task
    cmds: [task --list]
  build:
    desc: "  Build it  "
    cmds: [go build ./...]
  helper:
    internal: true
    cmds: [echo hidden]
  short: echo a task that's only a command
  list:
    - echo one
    - echo two
  explained:
    summary: |
      What it does, first.
      And then more.
  "with space":
    desc: An odd name
"#,
            ),
            ("docs/Taskfile.yml", "version: '3'\ntasks:\n  serve:\n    desc: Serve the docs\n"),
            ("lib/Tasks.yml", "version: '3'\ntasks:\n  vet: go vet ./...\n"),
            ("secret/Taskfile.yml", "version: '3'\ntasks:\n  leak: {desc: never shown}\n"),
        ],
    );
    let (cmds, err) = read_taskfile(&filepath::join(&[&dir, "Taskfile.yml"]));
    assert!(err.is_none(), "{err:?}");
    check(
        &cmds,
        &[
            "default=task default (List every task)",
            "build=task build (Build it)",
            "short=task short",
            "list=task list",
            "explained=task explained (What it does, first.)",
            "with space=task 'with space' (An odd name)",
            "docs:serve=task docs:serve (Serve the docs)",
            "vet=task vet",
        ],
    );
    for c in &cmds {
        assert_eq!(c.source, Source::Taskfile, "{} comes from {:?}, not the Taskfile", c.name, c.source);
    }
}

#[test]
fn taskfile_includes_each_other() {
    let (_t, dir) = tempdir();
    write(
        &dir,
        &[
            ("Taskfile.yml", "version: '3'\nincludes:\n  b: ./b\ntasks:\n  a: echo a\n"),
            ("b/Taskfile.yml", "version: '3'\nincludes:\n  back: ../Taskfile.yml\ntasks:\n  x: echo x\n"),
            ("bad/Taskfile.yml", "tasks: [not, a, map\n"),
        ],
    );
    let (cmds, err) = read_taskfile(&filepath::join(&[&dir, "Taskfile.yml"]));
    assert!(err.is_none(), "{err:?}");
    check(&cmds, &["a=task a", "b:x=task b:x"]);
    let (_, err) = read_taskfile(&filepath::join(&[&dir, "bad", "Taskfile.yml"]));
    assert!(err.is_some(), "a broken Taskfile reads without an error");
}

#[test]
fn package_json() {
    let (_t, dir) = tempdir();
    write(
        &dir,
        &[(
            "package.json",
            r#"{
  "name": "demo",
  "version": "1.0.0",
  "config": {"scripts": {"not": "these"}},
  "scripts": {
    "dev": "vite --host 127.0.0.1",
    "build": "vite build",
    "test:watch": "vitest",
    "odd": 42,
    "pre build": "echo quoted"
  },
  "dependencies": {"vite": "^7"}
}"#,
        )],
    );
    let (cmds, err) = read_package_json(&filepath::join(&[&dir, "package.json"]), &dir);
    assert!(err.is_none(), "{err:?}");
    check(
        &cmds,
        &[
            "dev=npm run dev (vite --host 127.0.0.1)",
            "build=npm run build (vite build)",
            "test:watch=npm run test:watch (vitest)",
            "pre build=npm run 'pre build' (echo quoted)",
        ],
    );
}

#[test]
fn package_manager() {
    for (name, files, want) in [
        (
            "its packageManager field",
            vec![
                ("app/package.json", r#"{"packageManager": "pnpm@11.9.0", "scripts": {"dev": "x"}}"#),
                ("app/yarn.lock", ""),
            ],
            "pnpm run dev",
        ),
        (
            "a lock file beside it",
            vec![("app/package.json", r#"{"scripts": {"dev": "x"}}"#), ("app/yarn.lock", "")],
            "yarn run dev",
        ),
        (
            "bun's lock file",
            vec![("app/package.json", r#"{"scripts": {"dev": "x"}}"#), ("app/bun.lock", "")],
            "bun run dev",
        ),
        (
            "a workspace's lock file above it",
            vec![("app/package.json", r#"{"scripts": {"dev": "x"}}"#), ("pnpm-lock.yaml", "")],
            "pnpm run dev",
        ),
        ("no lock file", vec![("app/package.json", r#"{"scripts": {"dev": "x"}}"#)], "npm run dev"),
        (
            "a manager it doesn't know",
            vec![
                ("app/package.json", r#"{"packageManager": "deno@2", "scripts": {"dev": "x"}}"#),
                ("app/package-lock.json", ""),
            ],
            "npm run dev",
        ),
    ] {
        let (_t, root) = tempdir();
        write(&root, &files);
        let (cmds, err) = read_package_json(&filepath::join(&[&root, "app", "package.json"]), &root);
        assert!(err.is_none(), "{name}: {err:?}");
        assert!(cmds.len() == 1 && cmds[0].run == want, "{name}: got {:?}, want {want}", brief(&cmds));
    }
}

#[test]
fn package_json_lock_above_the_repo() {
    let (_t, root) = tempdir();
    write(&root, &[("pnpm-lock.yaml", ""), ("repo/package.json", r#"{"scripts": {"dev": "x"}}"#)]);
    let (cmds, err) =
        read_package_json(&filepath::join(&[&root, "repo", "package.json"]), &filepath::join(&[&root, "repo"]));
    assert!(err.is_none(), "{err:?}");
    check(&cmds, &["dev=npm run dev (x)"]);
}

#[test]
fn package_json_broken() {
    let (_t, dir) = tempdir();
    for (name, body) in [
        ("not an object", r#"["dev"]"#),
        ("cut short", r#"{"scripts": {"dev": "vite""#),
        ("scripts a list", r#"{"scripts": ["dev"]}"#),
        ("no scripts at all", r#"{"name": "x"}"#),
    ] {
        write(&dir, &[("package.json", body)]);
        let (cmds, err) = read_package_json(&filepath::join(&[&dir, "package.json"]), &dir);
        if name == "no scripts at all" {
            assert!(err.is_none() && cmds.is_empty(), "{name}: got {:?}, {err:?}; want nothing", brief(&cmds));
            continue;
        }
        assert!(err.is_some(), "{name}: reads without an error: {:?}", brief(&cmds));
    }
}

#[test]
fn makefile() {
    let (_t, dir) = tempdir();
    write(
        &dir,
        &[(
            "Makefile",
            "# Variables aren't targets.
CC := gcc
SRC ::= main.c
FLAGS = -O2 -a:b
export PATH := $(PWD)/bin:$(PATH)
override X ?= 1

.PHONY: build test clean help
.DEFAULT_GOAL := build

build: deps ## Build the binary
\t$(CC) -o app $(SRC)

# Run the tests
test: build
\t./app --test # a comment in a recipe: not a target: no

lint fmt: ## Check the style
\t@echo $@

clean::
\trm -rf out

clean:: ## Remove what was built
\trm -rf more

%.o: %.c
\t$(CC) -c $<

$(OUT): $(SRC)
\ttouch $@

debug: FLAGS += -g
debug: build

define LONG
fake: target
endef

release: \\
\t\tbuild \\
\t\ttest ## Tag and push
\tgit tag

help:
\t@grep -E '^[a-z]+:.*##' $(MAKEFILE_LIST)
",
        )],
    );
    let (cmds, err) = read_makefile(&filepath::join(&[&dir, "Makefile"]));
    assert!(err.is_none(), "{err:?}");
    check(
        &cmds,
        &[
            "build=make build (Build the binary)",
            "test=make test (Run the tests)",
            "lint=make lint (Check the style)",
            "fmt=make fmt (Check the style)",
            "clean=make clean (Remove what was built)",
            "debug=make debug",
            "release=make release (Tag and push)",
            "help=make help",
        ],
    );
}

#[test]
fn load_reads_the_folders_files() {
    let (_t, dir) = tempdir();
    write(
        &dir,
        &[
            ("Makefile", "all: ## Everything\n\ttrue\n"),
            ("package.json", r#"{"scripts": {"start": "node ."}}"#),
            ("taskfile.yml", "version: '3'\ntasks:\n  up: {desc: Up}\n"),
        ],
    );
    let files = load(&dir, &dir);
    let mut got = Vec::new();
    for f in &files {
        assert!(f.err.is_none(), "{}: {:?}", f.path, f.err);
        got.push(format!("{}: {}", f.source.label(), brief(&f.commands).join(", ")));
    }
    assert_eq!(
        got,
        [
            "Taskfile: up=task up (Up)",
            "package.json: start=npm run start (node .)",
            "Makefile: all=make all (Everything)"
        ]
    );
    let (_e, empty) = tempdir();
    let files = load(&empty, &dir);
    assert!(files.is_empty(), "an empty folder has {} project files", files.len());
}

#[test]
fn load_keeps_the_others_when_one_is_broken() {
    let (_t, dir) = tempdir();
    write(&dir, &[("package.json", "{"), ("Makefile", "all:\n\ttrue\n")]);
    let files = load(&dir, &dir);
    assert!(
        files.len() == 2 && files[0].err.is_some() && files[1].err.is_none() && files[1].commands.len() == 1,
        "got {files:?}"
    );
}

#[test]
fn root_is_the_nearest_project() {
    let (_t, home) = tempdir();
    write(
        &home,
        &[
            ("projects/app/package.json", "{}"),
            ("projects/app/src/components/x.ts", ""),
            ("projects/app/packages/ui/package.json", "{}"),
            ("projects/app/packages/ui/src/y.ts", ""),
            ("projects/bare/.git/HEAD", ""),
            ("projects/bare/src/z.go", ""),
            ("Makefile", "all:\n"),
        ],
    );
    let p = |s: &str| filepath::join(&[&home, s]);
    for (input, want) in [
        ("projects/app", "projects/app"),
        ("projects/app/src/components", "projects/app"),
        ("projects/app/packages/ui/src", "projects/app/packages/ui"),
        ("projects/bare/src", "projects/bare/src"), // nothing up to the repository's root
        ("projects", "projects"),                   // not up to the home folder's Makefile
    ] {
        assert_eq!(root(&p(input), &home), p(want), "root({input})");
    }
    assert_eq!(repo_root(&p("projects/bare/src")), p("projects/bare"));
}

#[test]
fn changed_notices_an_edit() {
    let (_t, dir) = tempdir();
    let before = changed(&dir, &[]);
    write(&dir, &[("Makefile", "all:\n")]);
    assert_ne!(changed(&dir, &[]), before, "a new Makefile doesn't count as a change");
    write(&dir, &[("apps/web/package.json", "{}")]);
    let files = load(&dir, &dir);
    let before = changed(&dir, &files);
    let future = SystemTime::now() + Duration::from_secs(3600);
    let f = fs::File::options().write(true).open(filepath::join(&[&dir, "apps", "web", "package.json"])).unwrap();
    f.set_modified(future).unwrap();
    assert_ne!(changed(&dir, &files), before, "an edit to a folder's package.json doesn't count as a change");
}

// subfolders_test.go

/// where is a file as load names it: its tab, its label and its first
/// command, for comparing.
fn where_(files: &[File]) -> Vec<String> {
    files
        .iter()
        .map(|f| {
            let first = f.commands.first().map(|c| c.run.as_str()).unwrap_or("");
            format!("{} {} [{}] {}", f.source.label(), f.rel_path(), f.label, first)
        })
        .collect()
}

#[test]
fn load_looks_in_a_monorepos_folders() {
    let (_t, dir) = tempdir();
    write(
        &dir,
        &[
            (".git/HEAD", ""),
            ("package.json", r#"{"scripts": {"lint": "eslint ."}}"#),
            ("pnpm-lock.yaml", ""),
            ("Taskfile.yml", "version: '3'\nincludes:\n  docs: ./docs\ntasks:\n  up: {desc: Up}\n"),
            ("docs/Taskfile.yml", "version: '3'\ntasks:\n  serve: {desc: Serve the docs}\n"),
            ("apps/web/package.json", r#"{"scripts": {"dev": "next dev"}}"#),
            ("apps/api/compose.yaml", "services:\n  db: {image: postgres:16}\n"),
            ("apps/api/Makefile", "run:\n\tgo run .\n"),
            ("packages/ui/package.json", r#"{"scripts": {"build": "tsc"}}"#),
            ("a/b/c/package.json", r#"{"scripts": {"deep": "true"}}"#),
            ("a/b/c/d/package.json", r#"{"scripts": {"too-deep": "true"}}"#),
            ("node_modules/x/package.json", r#"{"scripts": {"theirs": "true"}}"#),
            ("apps/web/node_modules/y/package.json", r#"{"scripts": {"theirs": "true"}}"#),
            (".hidden/package.json", r#"{"scripts": {"hidden": "true"}}"#),
            ("dist/package.json", r#"{"scripts": {"built": "true"}}"#),
            ("target/pom.xml", "<project/>"),
        ],
    );
    let got = where_(&load(&dir, &dir));
    let want = [
        "Taskfile Taskfile.yml [] task up",
        "package.json package.json [] pnpm run lint",
        "package.json a/b/c/package.json [a/b/c] pnpm run deep",
        "Makefile apps/api/Makefile [apps/api] make run",
        "Compose apps/api/compose.yaml [apps/api] docker compose up",
        "package.json apps/web/package.json [apps/web] pnpm run dev",
        "package.json packages/ui/package.json [packages/ui] pnpm run build",
    ];
    assert_eq!(got, want, "got\n  {}\nwant\n  {}", got.join("\n  "), want.join("\n  "));
    for f in load(&dir, &dir) {
        let want = filepath::join(&[&dir, &f.rel]);
        assert_eq!(f.dir, want, "{} runs in {}, not {want}", f.rel_path(), f.dir);
    }
}

#[test]
fn load_stays_in_a_folder_thats_no_project() {
    let (_t, dir) = tempdir(); // ~/projects, say: no project file, no repository
    write(&dir, &[("one/package.json", "{}"), ("two/Makefile", "all:\n")]);
    let files = load(&dir, &dir);
    assert!(files.is_empty(), "it looked in every project under it: {:?}", where_(&files));
}

#[test]
fn compose() {
    let (_t, dir) = tempdir();
    write(
        &dir,
        &[
            (
                "compose.yaml",
                r#"services:
  db:
    image: postgres:16
  api:
    build: ./api
  web:
    build:
      context: ./web
      dockerfile: Dockerfile.dev
  worker:
    build: {dockerfile: Worker.Dockerfile}
  "odd one": {}
"#,
            ),
            ("docker-compose.yml", "services:\n  old: {image: busybox}\n"),
            ("docker-compose.dev.yml", "services:\n  dev: {image: node:22}\n"),
            ("docker-compose.override.yml", "services:\n  db: {ports: ['5432:5432']}\n"),
            ("composer.json", "{}"),
        ],
    );
    let files = load(&dir, &dir);
    assert_eq!(
        where_(&files),
        [
            "Compose compose.yaml [] docker compose up",
            "Compose docker-compose.dev.yml [docker-compose.dev.yml] docker compose -f docker-compose.dev.yml up",
            "Compose docker-compose.yml [docker-compose.yml] docker compose -f docker-compose.yml up",
        ]
    );
    check(
        &files[0].commands,
        &[
            "up=docker compose up (Start every service; their logs follow here, and s stops them)",
            "up --build=docker compose up --build (Build the images again, then start every service)",
            "down=docker compose down (Stop the services and remove their containers and networks)",
            "ps=docker compose ps (The services' containers, and how they are)",
            "logs -f=docker compose logs -f (Follow the services' logs, however they were started)",
            "up db=docker compose up db (Start db and what it depends on: image postgres:16)",
            "up api=docker compose up api (Start api and what it depends on: built from ./api)",
            "up web=docker compose up web (Start web and what it depends on: built from ./web)",
            "up worker=docker compose up worker (Start worker and what it depends on: built from .)",
            "up 'odd one'=docker compose up 'odd one' (Start odd one and what it depends on)",
        ],
    );
    let (_, err) = read_compose(&filepath::join(&[&dir, "composer.json"]), false);
    assert!(err.is_none(), "a file without services: {err:?}");
    write(&dir, &[("compose.yaml", "services: [broken")]);
    let (_, err) = read_compose(&filepath::join(&[&dir, "compose.yaml"]), false);
    assert!(err.is_some(), "a broken compose file reads without an error");
}

const PARENT_POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <artifactId>site</artifactId>
  <packaging>pom</packaging>
  <modules>
    <module>core</module>
    <module>ui.apps</module>
  </modules>
  <profiles>
    <profile>
      <id>adobe-public</id>
      <activation><activeByDefault>true</activeByDefault></activation>
    </profile>
    <profile><id>autoInstallSinglePackage</id></profile>
    <profile><id>autoInstallBundle</id></profile>
    <profile><id>integration</id></profile>
  </profiles>
</project>"#;

#[test]
fn pom() {
    let (_t, dir) = tempdir();
    write(
        &dir,
        &[
            (".git/HEAD", ""),
            ("pom.xml", PARENT_POM),
            ("mvnw", "#!/bin/sh\n"),
            (
                "core/pom.xml",
                "<project><profiles><profile><id>autoInstallBundle</id></profile><profile><id>coverage</id></profile></profiles></project>",
            ),
            (
                "ui.apps/pom.xml",
                "<project><profiles><profile><id>autoInstallPackage</id></profile><profile><id>coverage</id></profile></profiles></project>",
            ),
            ("ui.frontend/package.json", r#"{"scripts": {"build": "webpack"}}"#),
        ],
    );
    let files = load(&dir, &dir);
    check(
        &files[0].commands,
        &[
            "clean install=./mvnw clean install (Build every module and install it in ~/.m2)",
            "clean verify=./mvnw clean verify (Build every module and run the tests, installing nothing)",
            "test=./mvnw test (Run the unit tests)",
            "clean install -PautoInstallSinglePackage=./mvnw clean install -PautoInstallSinglePackage (Build it all and deploy it as one package to AEM's author (localhost:4502 unless aem.host and aem.port say otherwise))",
            "clean install -PautoInstallBundle=./mvnw clean install -PautoInstallBundle (Build and deploy the OSGi bundle to AEM's author)",
            "clean install -Pintegration=./mvnw clean install -Pintegration (clean install with the integration profile)",
            "clean install -Pcoverage=./mvnw clean install -Pcoverage (clean install with the coverage profile (core, ui.apps))",
            "clean install -PautoInstallPackage=./mvnw clean install -PautoInstallPackage (Build and deploy each content package to AEM's author (ui.apps))",
        ],
    );
    assert_eq!(
        where_(&files[1..]),
        [
            "Maven core/pom.xml [core] ../mvnw clean install",
            "Maven ui.apps/pom.xml [ui.apps] ../mvnw clean install",
            "package.json ui.frontend/package.json [ui.frontend] npm run build",
        ]
    );
    assert_eq!(files[1].commands[0].desc, "Build it and install it in ~/.m2", "a module says");

    let (_b, bare) = tempdir();
    write(&bare, &[("pom.xml", "<project/>")]);
    let (cmds, err) = read_pom(&filepath::join(&[&bare, "pom.xml"]), &bare);
    assert!(err.is_none() && cmds[0].run == "mvn clean install", "without a wrapper: {:?}, {err:?}", brief(&cmds));
    write(&bare, &[("pom.xml", "<project><modules>")]);
    let (_, err) = read_pom(&filepath::join(&[&bare, "pom.xml"]), &bare);
    assert!(err.is_some(), "a broken pom reads without an error");
}

#[test]
fn root_finds_compose_and_maven() {
    let (_t, home) = tempdir();
    write(
        &home,
        &[
            ("aem/pom.xml", "<project/>"),
            ("aem/core/src/x.java", ""),
            ("stack/compose.yml", "services: {}"),
            ("stack/conf/nginx.conf", ""),
        ],
    );
    let cases: BTreeMap<&str, &str> = [("aem/core/src", "aem"), ("stack/conf", "stack")].into_iter().collect();
    for (input, want) in cases {
        assert_eq!(root(&filepath::join(&[&home, input]), &home), filepath::join(&[&home, want]), "root({input})");
    }
}

#[test]
fn duplicate_script_keys_keep_the_final_value_in_one_native_operation() {
    let (_temp, dir) = tempdir();
    write(&dir, &[("package.json", r#"{"scripts":{"build":"first","test":"check","build":"last"}}"#)]);
    let (commands, error) = read_package_json(&filepath::join(&[&dir, "package.json"]), &dir);
    assert!(error.is_none());
    check(&commands, &["build=npm run build (last)", "test=npm run test (check)"]);
}
