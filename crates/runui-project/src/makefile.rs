//! A Makefile's targets, with what they say they do.

use std::sync::LazyLock;

use regex::Regex;
use runui_core::golang::go_lossy;

use crate::{Command, Source, quote};

/// MAKEFILE_NAMES are the names GNU make looks for, in its order.
pub(crate) const MAKEFILE_NAMES: [&str; 3] = ["GNUmakefile", "makefile", "Makefile"];

// RE2's \s is ASCII's whitespace, and its \b an ASCII word's boundary:
// here they're spelled out, the regex crate's being Unicode's.

/// ASSIGN_LINE sets a variable: X = 1, X := 1, X ::= 1, X += 1, export X ?= 1.
static ASSIGN_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^[\t\n\f\r ]*(?:(?:export|override|private)[\t\n\f\r ]+)*[^:#=\t\n\f\r ]+[\t\n\f\r ]*(?::{1,3}|[+?!])?=",
    )
    .expect("a regexp")
});

/// RULE_LINE is a rule: its targets, a colon or two, and the rest.
static RULE_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([^:#=]+?)[\t\n\f\r ]*::?(.*)$").expect("a regexp"));

/// TARGET_VAR is a target-specific variable after a rule's colon:
/// build: CFLAGS += -O2.
static TARGET_VAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^[\t\n\f\r ]*(?:export[\t\n\f\r ]+|override[\t\n\f\r ]+)?[A-Za-z0-9_.-]+[\t\n\f\r ]*(?::|::|\+|\?|!)?=",
    )
    .expect("a regexp")
});

/// DEFINE_LINE starts a multi-line variable, which ends at endef.
static DEFINE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\t\n\f\r ]*(?:(?:export|override)[\t\n\f\r ]+)*define(?-u:\b)").expect("a regexp"));

/// MAX_TOKEN is the longest line bufio.Scanner reads with the buffer
/// ReadMakefile gives it.
const MAX_TOKEN: usize = 1024 * 1024;

/// read_makefile is a Makefile's targets in the order it has them, each
/// with what it says it does: a ## comment after the rule (the
/// self-documenting Makefile's way), else the comment right above it.
/// Special targets (.PHONY), pattern rules (%.o) and targets made of
/// variables are left out.
pub fn read_makefile(path: &str) -> (Vec<Command>, Option<anyhow::Error>) {
    read_tracked(path, &mut crate::discovery::Scan::new(&runui_core::golang::filepath::dir(path)))
}

pub(crate) fn read_tracked(path: &str, scan: &mut crate::discovery::Scan) -> (Vec<Command>, Option<anyhow::Error>) {
    let data = match scan.read(path) {
        Ok(data) => data,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let mut cmds: Vec<Command> = Vec::new();
    let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut comment = String::new();
    let mut define = false;
    let mut line = String::new();
    let (lines, err) = scan_lines(&data);
    for text in lines {
        let mut text = text;
        if text.ends_with('\\') && (!line.is_empty() || !text.starts_with('\t')) {
            line.push_str(&text[..text.len() - 1]);
            line.push(' ');
            continue;
        }
        line.push_str(&text);
        text = std::mem::take(&mut line);
        if define {
            define = text.trim() != "endef";
            continue;
        } else if DEFINE_LINE.is_match(&text) {
            (define, comment) = (true, String::new());
            continue;
        } else if text.starts_with('\t') {
            comment.clear();
            continue;
        } else if text.trim().starts_with('#') {
            comment = text.trim().trim_start_matches('#').trim().to_string();
            continue;
        } else if text.trim().is_empty() {
            comment.clear();
            continue;
        }
        let above = std::mem::take(&mut comment);
        let (body, desc) = text.split_once("##").unwrap_or((&text, ""));
        let mut desc = desc.trim().to_string();
        if desc.is_empty() {
            desc = above;
        }
        let body = body.find('#').map_or(body, |hash| &body[..hash]);
        if ASSIGN_LINE.is_match(body) {
            continue;
        }
        let Some(m) = RULE_LINE.captures(body) else { continue };
        if TARGET_VAR.is_match(&m[2]) {
            continue;
        }
        for name in m[1].split_whitespace() {
            if name.starts_with('.') || name.contains(['%', '$', '(', ')']) {
                continue;
            }
            if let Some(&i) = at.get(name) {
                if cmds[i].desc.is_empty() {
                    cmds[i].desc = desc.clone();
                }
                continue;
            }
            at.insert(name.to_string(), cmds.len());
            cmds.push(Command {
                source: Source::Make,
                name: name.to_string(),
                run: format!("make {}", quote(name)),
                desc: desc.clone(),
                argv: vec!["make".into(), name.into()],
                args_separator: false,
                defining_file: path.into(),
                defining_line: None,
                variant: String::new(),
                generated: false,
                aliases: Vec::new(),
                hook: false,
                requirements: Vec::new(),
            });
        }
    }
    (cmds, err)
}

/// scan_lines splits data into lines as bufio.Scanner's ScanLines does:
/// at each newline, a carriage return before it dropped, the last line
/// kept without one; a line too long for the scanner's buffer stops it,
/// with its error.
fn scan_lines(data: &[u8]) -> (Vec<String>, Option<anyhow::Error>) {
    let mut out = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let (line, next) = match rest.iter().position(|&c| c == b'\n') {
            Some(i) if i < MAX_TOKEN => (&rest[..i], &rest[i + 1..]),
            None if rest.len() < MAX_TOKEN => (rest, &rest[rest.len()..]),
            _ => return (out, Some(anyhow::anyhow!("bufio.Scanner: token too long"))),
        };
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        out.push(go_lossy(line));
        rest = next;
    }
    (out, None)
}
