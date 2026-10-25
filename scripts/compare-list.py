#!/usr/bin/env python3
"""Runs the Go and the Rust runui on the same project folders and compares
what `runui list` prints, byte for byte: hand-written cases (a Taskfile's,
package.json's, a Makefile's, a compose file's and a pom's edge cases and
broken files, monorepos, the flags), then each kind of project file mutated
at random from a seed.

    RUNUI_COMPARE=dir python3 scripts/compare-list.py [seed] [mutations per kind]

dir holds runui-go and runui-rs (task compare builds them there); the
projects, homes and state are made under it."""

import os
import random
import shutil
import subprocess
import sys

ROOT = os.environ.get("RUNUI_COMPARE") or os.path.join(os.environ.get("TMPDIR", "/tmp"), "runui-compare")
GO = os.path.join(ROOT, "runui-go")
RS = os.path.join(ROOT, "runui-rs")
HOME = os.path.join(ROOT, "home")
FIX = os.path.join(HOME, "fixtures")

env = dict(os.environ)
env.update(HOME=HOME, XDG_CONFIG_HOME=os.path.join(HOME, "config"), XDG_STATE_HOME=os.path.join(HOME, "state"))


def run(binary, args, cwd=HOME):
    p = subprocess.run([binary] + args, cwd=cwd, env=env, capture_output=True, timeout=30)
    return p.returncode, p.stdout, p.stderr


def write(dir, files):
    for name, body in files.items():
        path = os.path.join(dir, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(body if isinstance(body, bytes) else body.encode())


failures = 0
checked = 0
not_utf8 = 0


def go_lossy(b):
    """b as the Rust build reads text that isn't UTF-8: each byte that
    doesn't start a character U+FFFD, as Go's utf8.DecodeRune reads them.
    The Go build keeps such bytes as they are (a Makefile's, an XML name in
    an error); Rust's strings can't."""
    out, i = bytearray(), 0
    while i < len(b):
        c = b[i]
        n = 1 if c < 0x80 else 2 if 0xC2 <= c <= 0xDF else 3 if 0xE0 <= c <= 0xEF else 4 if 0xF0 <= c <= 0xF4 else 0
        try:
            if n == 0:
                raise UnicodeDecodeError("utf-8", b, i, i + 1, "")
            b[i : i + n].decode("utf-8")
            out += b[i : i + n]
            i += n
        except UnicodeDecodeError:
            out += "\ufffd".encode()
            i += 1
    return bytes(out)


def compare(name, args, cwd=HOME):
    global failures, checked
    checked += 1
    global not_utf8
    g, r = run(GO, args, cwd), run(RS, args, cwd)
    # MCP is a Rust-only command. Keep its one help line visible to users,
    # then compare every other byte of Go's CLI output as before.
    mcp_line = b"  runui mcp [-C DIR]    serve project commands and runs to an MCP client over stdio\n"
    if b"Usage:\n" in g[1] + g[2]:
        normalized = (r[0], r[1].replace(mcp_line, b"", 1), r[2].replace(mcp_line, b"", 1))
    else:
        normalized = r
    if g != normalized and (g[0], go_lossy(g[1]), go_lossy(g[2])) == normalized:
        not_utf8 += 1
        return
    if g != normalized:
        failures += 1
        if failures <= 25:
            print(f"--- {name}: {args}")
            for label, (code, out, err) in (("go", g), ("rs", r)):
                print(f"  {label} exit {code}")
                print("  out:", out.decode(errors="replace").replace("\n", "\n       "))
                print("  err:", err.decode(errors="replace").replace("\n", "\n       "))


def case(name, files, args=None):
    d = os.path.join(FIX, name)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    write(d, files)
    compare(name, args or ["list", "-C", d])


shutil.rmtree(HOME, ignore_errors=True)
os.makedirs(FIX)

# ---- hand-written cases

TASKFILES = {
    "plain": "version: '3'\ntasks:\n  a: echo a\n  b:\n    desc: B it\n",
    "broken": "tasks: [not, a, map\n",
    "seq doc": "- a\n- b\n",
    "scalar doc": "hello\n",
    "empty": "",
    "only comment": "# nothing\n",
    "dup tasks": "tasks:\n  a: x\ntasks:\n  b: y\n",
    "dup task key": "tasks:\n  a:\n    desc: one\n    desc: two\n  b: y\n",
    "alias task": "x: &t {desc: shared}\ntasks:\n  a: *t\n  b:\n    <<: *t\n    summary: s\n",
    "merge seq": "a: &a {desc: A}\nb: &b {internal: true}\ntasks:\n  t:\n    <<: [*a, *b]\n  u:\n    <<: *a\n",
    "bad merge": "tasks:\n  t:\n    <<: 5\n",
    "binary": "tasks:\n  t:\n    desc: !!binary aGVsbG8gd29ybGQ=\n  u:\n    desc: !!binary '###'\n",
    "typed desc": "tasks:\n  n: {desc: 42}\n  b: {desc: true}\n  f: {desc: 1.5}\n  l: {desc: [a, b]}\n  m: {desc: {a: b}}\n  z: {desc: ~}\n  t: {desc: 2001-12-14}\n",
    "yaml11 bools": "tasks:\n  a: {internal: yes}\n  b: {internal: 'yes'}\n  c: {internal: on}\n  d: {internal: 1}\n  e: {internal: NO}\n",
    "keys": "tasks:\n  1: one\n  true: t\n  ~: n\n  1.5: f\n  0x10: h\n  'quoted key': q\n  \"dq\": d\n  ? complex\n  : v\n",
    "summary": "tasks:\n  a:\n    summary: |\n      first\n      second\n  b:\n    summary: \"  spaced  \\n next\"\n  c:\n    desc: '   '\n    summary: fallback\n",
    "tabs": "tasks:\n\ta: x\n",
    "flow": "{tasks: {a: {desc: A}, b: echo}}\n",
    "anchors loop": "tasks: &x\n  a: *x\n",
    "unicode": "tasks:\n  héllo:\n    desc: ünïcode ✓\n  日本: x\n",
    "multi doc": "tasks:\n  a: x\n---\ntasks:\n  b: y\n",
    "bom": "\ufefftasks:\n  a: x\n",
    "crlf": "tasks:\r\n  a:\r\n    desc: crlf\r\n",
    "deep": "tasks:\n  a:\n    cmds:\n      - task: b\n    desc: >\n      folded\n      text\n",
    "includes": "includes:\n  x: ./inc\n  y:\n    taskfile: inc/Taskfile.yml\n    flatten: true\n  z: [1]\n  w: {taskfile: 5}\n  v: ~\n  u:\n    taskfile: ./missing\n  t:\n    taskfile: ./missing2\n    optional: yes\ntasks:\n  top: x\n",
}
for n, body in TASKFILES.items():
    files = {"Taskfile.yml": body}
    if n == "includes":
        files["inc/Taskfile.yml"] = "tasks:\n  i: x\n"
    case("task-" + n.replace(" ", "-"), files)

PACKAGES = [
    "{", '["dev"]', '{"scripts": {"dev": "vite"', '{"scripts": ["dev"]}', '{"name": "x"}', "", "  ", "x",
    '{"a" 1}', '{"a": 1 "b": 2}', '{"scripts": {"a": "b",}}', '{"scripts": {"a": "b"}} trailing',
    '{"scripts": {"a": "b"}', '{"scripts": {"a": tru}}', r'{"scripts": {"a": "b\x01"}}', '{"x": [1, 2,]}',
    '{"x": {"y": }}', '{"scripts": {"a": "b", "c": 1, "d": null, "e": "f", "g": [1], "h": {"i": "j"}}}',
    '{"packageManager": 5, "scripts": {"a": "b"}}', '{"scripts": {"a": "b"}, "scripts": {"c": "d"}}', '{1: 2}',
    r'{"x": "\u12"}', '{"x": 01}', '{"x": -}', '{"x": 1.}', '{"x": 1e}', '{"x": nul}', '{"x": "a', '{"x": [',
    '{"x": [1', '{"x"', '{"x":', '{"scripts": {"a"', '{"scripts": {"a":', '{"scripts": {"a": "b",',
    '{"scripts": {"a": "b" "c"}}', b'{"scripts": {"a": "\xff\xfe"}}', b'{"scripts": {"\xe2\x82": "x"}}',
    r'{"scripts": {"aA": "b\u00e9\ud83d\ude00\ud800x"}}', '{"packageManager": "yarn@4", "scripts": {"x y": "z"}}',
    '{"packageManager": null, "scripts": {"a": "b"}}', '{"scripts": null}', '{"scripts": {}}', "null", "[]",
    '"str"', "42", '{"scripts": {"a": "\t"}}', '{"scripts": {"a": "b"}}{"x": 1}', '{"x": [1, ]}', '{"x": {,}}',
    '{,}', '{"x": 1,}', '{"scripts": {"a": "b"},}', '{"x": "\n"}', '{"x": 1e+}', '{"x": -01}', '{"x": .5}',
    '{"x": tRue}', '{"x": [}', '{"x": ]}', '{"x": }', '{"x" : [1] ]', '{"a": "b"} ]', '{"scripts" "x"}',
    '{"scripts": {"a": "b"} "x": 1}', '\t{\n"scripts":\r\n{"a":"b"}}', '{"x": 1E5, "y": -0.5e-3, "scripts": {"n": "1"}}',
]
for i, body in enumerate(PACKAGES):
    case(f"npm-{i}", {"package.json": body})

MAKEFILES = [
    "all:\n\ttrue\n", "a b c: ## three\n", "x = 1\ny := 2\n", "export A=1\nunexport B\n", ".PHONY: a\na:\n",
    "a:: ## first\na:: ## second\n", "# above\nb: c\n", "# above\n\nb: c\n", "c: d # comment\n",
    "d: ## ## double\n", "e:\\\n  f ## continued\n", "\tnot a rule:\n", "define X\na: b\nendef\nz:\n",
    "override define Y\nq:\nendef\n", "$(X): y\n", "%.o: %.c\n", "a: B = 1\n", "a: export C := 2\n",
    "ifeq ($(A),1)\nb:\nendif\n", "include other.mk\n-include x\n", "a:b\n", "a=b:c\n", "a : b\n",
    "a\\:b: c\n", "ünï: code\n", "a: ;echo inline\n", "a:\r\n\techo\r\n", "a: \\\n", "\\\n", "x:\\",
    "a := $(shell echo x:y)\n", "a ?= b\nc != d\ne += f\n", "private x = 1\n", "a::=b\n", "a:::=b\n",
    "a: b\n\t@echo $@ ## not this\n", "   indented: x\n", "a: ## \n", "#\n#\na:\n", "## only\na:\n",
]
for i, body in enumerate(MAKEFILES):
    case(f"make-{i}", {"Makefile": body})

COMPOSES = [
    "services:\n  db: {image: postgres}\n", "services: [broken", "services: []\n", "services: ~\n", "x: 1\n",
    "services:\n  a:\n    build: .\n    image: x\n  b:\n    build:\n      dockerfile: D\n  c:\n    build: ~\n  d: ~\n  e: [1]\n  f:\n    image: [x]\n  g:\n    build: {context: [x]}\n",
    "services:\n  'odd one': {}\n  \"x y\": {image: z}\n", "", "- a\n", "services:\n  a: &a {image: i}\n  b: *a\n",
    "services:\n  a:\n    <<: {image: merged}\n", "services:\n  a:\n    image: i\n    image: j\n",
]
for i, body in enumerate(COMPOSES):
    case(f"compose-{i}", {"compose.yaml": body})
case("compose-names", {"compose.yml": "services: {}", "docker-compose.prod.yaml": "services: {a: {}}",
                       "compose.override.yml": "x", "docker-compose.yml": "services: {b: {}}"})

POMS = [
    "<project/>", "<project><modules>", "", "<foo/>", "<?xml version='1.0'?><project></project>",
    "<project><modules><module>a</module><module> b </module><other>c</other></modules></project>",
    "<project><profiles><profile><id> x </id></profile><profile><id></id></profile><profile><id>y</id><activation><activeByDefault> true </activeByDefault></activation></profile></profiles></project>",
    "<project><profiles><profile><id>a</id><id>b</id></profile></profiles></project>",
    "<project><profiles><profile><id>a</id><activation><activeByDefault>true</activeByDefault></activation><activation><x/></activation></profile></profiles></project>",
    "<project><profiles><profile><id>p</id></profile><profile><id>p</id></profile></profiles></project>",
    "<project xmlns='http://maven.apache.org/POM/4.0.0'><modules><module>m</module></modules></project>",
    "<x:project xmlns:x='u'/>", "<project>&amp;&bogus;</project>", "<project><a></b></project>",
    "<?xml version='1.0' encoding='ISO-8859-1'?><project/>", "<!-- c --><project/>", "garbage",
    "<project><profiles><profile><id>a<!-- c -->b</id></profile></profiles></project>",
    "<project><profiles><profile><id><![CDATA[cd]]></id></profile></profiles></project>",
    "<project><profiles><profile><id>a<x>y</x>b</id></profile></profiles></project>",
    "<project/><extra", "<project attr='x'><modules/></project>", "<project><modules><module>a</module>",
]
for i, body in enumerate(POMS):
    case(f"pom-{i}", {"pom.xml": body})
case("pom-modules", {
    "pom.xml": "<project><modules><module>core</module><module>missing</module><module>web/ui</module></modules><profiles><profile><id>own</id></profile></profiles></project>",
    "core/pom.xml": "<project><modules><module>sub</module></modules><profiles><profile><id>own</id></profile><profile><id>c</id></profile><profile><id>c</id></profile></profiles></project>",
    "core/sub/pom.xml": "<project><profiles><profile><id>deep</id></profile></profiles></project>",
    "web/ui/pom.xml": "<project><profiles><profile><id>autoInstallPackage</id></profile><profile><id>c</id></profile></profiles></project>",
    "mvnw": "#!/bin/sh\n",
})

# monorepos and roots
case("mono", {
    ".git/HEAD": "", "package.json": '{"scripts": {"lint": "eslint ."}}', "pnpm-lock.yaml": "",
    "Taskfile.yml": "version: '3'\nincludes:\n  docs: ./docs\ntasks:\n  up: {desc: Up}\n",
    "docs/Taskfile.yml": "tasks:\n  serve: {desc: Serve}\n", "apps/web/package.json": '{"scripts": {"dev": "next"}}',
    "apps/api/compose.yaml": "services:\n  db: {image: pg}\n", "apps/api/Makefile": "run:\n\tgo run .\n",
    "a/b/c/package.json": '{"scripts": {"deep": "1"}}', "a/b/c/d/package.json": '{"scripts": {"no": "1"}}',
    "node_modules/x/package.json": "{}", ".hidden/Makefile": "x:\n", "vendor/Makefile": "x:\n",
    "apps/web/yarn.lock": "", "apps/web/bun.lockb": "",
})
case("mono-src", {"package.json": '{"scripts": {"a": "b"}}', "src/deep/x.ts": ""},
     ["list", "-C", os.path.join(FIX, "mono-src", "src", "deep")])
case("empty-folder", {"README": "x"})
case("files-and-folders", {"Makefile": "a:\n", "GNUmakefile": "g:\n", "makefile": "m:\n",
                           "taskfile.yml": "tasks: {t: x}", "Taskfile.dist.yml": "tasks: {d: x}"})

# runs, and ps with nothing run
compare("ps-empty", ["ps"])
compare("help", ["help"])
compare("-h", ["-h"])
compare("--help", ["--help"])
compare("usage", ["nope"])
compare("flag", ["-x"])
compare("flag-eq", ["-C=" + os.path.join(FIX, "mono"), "list"])
compare("flag-after", ["list", "-C", os.path.join(FIX, "mono"), "extra"])
compare("flag-dashdash", ["list", "--", "x"])
compare("flag-noarg", ["list", "-C"])
compare("flag-bad", ["list", "---C", "x"])
compare("flag-double", ["list", "--C", os.path.join(FIX, "mono")])
compare("supervise", ["_supervise"])
compare("supervise-two", ["_supervise", "a", "b"])
compare("supervise-none", ["_supervise", os.path.join(FIX, "nowhere")])
compare("cmd-after-flag", ["-C", os.path.join(FIX, "mono"), "list"])
compare("list-missing", ["list", "-C", os.path.join(FIX, "does-not-exist")])

print(f"hand-written: {checked} checked, {failures} differ")

# ---- random mutations of each kind of file

SEEDS = {
    "Taskfile.yml": TASKFILES["plain"] + TASKFILES["summary"].replace("tasks:\n", "") + "includes:\n  x: ./inc\n",
    "package.json": '{"name": "x", "packageManager": "pnpm@9", "scripts": {"dev": "vite --host", "a b": "c", "n": 1, "z": null}, "deps": {"a": [1, 2.5e3, true, false, null, {"q": "\\u00e9\\n"}]}}',
    "Makefile": "".join(MAKEFILES[:12]),
    "compose.yaml": COMPOSES[5],
    "pom.xml": POMS[6].replace("</project>", "<modules><module>m</module></modules></project>"),
}
ALPHABET = list('{}[]:," \n\t\\-0123456789.eEtrufalsn#=@<>/!&*~?%$()xyz\'') + ["\u00e9", "\xff"]
rng = random.Random(int(sys.argv[1]) if len(sys.argv) > 1 else 42)
per_kind = int(sys.argv[2]) if len(sys.argv) > 2 else 300
before = failures
mutated = 0
for name, seed in SEEDS.items():
    for i in range(per_kind):
        s = list(seed)
        for _ in range(rng.randint(1, 4)):
            op = rng.random()
            at = rng.randrange(len(s) + 1)
            if op < 0.4 and s:
                del s[min(at, len(s) - 1)]
            elif op < 0.7:
                s.insert(at, rng.choice(ALPHABET))
            elif s:
                s[min(at, len(s) - 1)] = rng.choice(ALPHABET)
            if rng.random() < 0.15:
                s = s[: rng.randrange(len(s) + 1)]
        body = "".join(s).encode("utf-8", errors="surrogateescape")
        body = body.replace("\xff".encode(), b"\xff")
        d = os.path.join(FIX, "mut", f"{name}-{i}")
        os.makedirs(d, exist_ok=True)
        write(d, {name: body})
        if name == "Taskfile.yml":
            write(d, {"inc/Taskfile.yml": "tasks: {i: x}"})
        if name == "pom.xml":
            write(d, {"m/pom.xml": "<project><profiles><profile><id>mp</id></profile></profiles></project>"})
        mutated += 1
        compare(f"mut {name} {i}", ["list", "-C", d])
print(f"mutations: {mutated} checked, {failures - before} differ")
print(f"total: {checked} checked, {failures} differ")
if not_utf8:
    print(f"(and {not_utf8} the same but for bytes that aren't UTF-8, which the Go build prints as they are and this one as U+FFFD)")
sys.exit(1 if failures else 0)
