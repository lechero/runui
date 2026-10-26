use super::{Dialect, MAX_COMMAND_BYTES, Record};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

// Bound allocation even for millions of tiny or duplicate declarations. The
// final distinct-entry limit is applied separately after format validation.
const MAX_RECORDS: usize = 10_000;
#[derive(Default)]
struct Records {
    values: VecDeque<Record>,
    clipped: bool,
}
impl Records {
    fn push(&mut self, record: Record) {
        if self.values.len() == MAX_RECORDS {
            self.values.pop_front();
            self.clipped = true;
        }
        self.values.push_back(record);
    }
    fn finish(self, notes: &mut Vec<String>) -> Vec<Record> {
        if self.clipped {
            notes.push(format!("Only the latest {MAX_RECORDS} source records were considered before deduplication."));
        }
        self.values.into_iter().collect()
    }
}

pub(super) fn parse(dialect: Dialect, bytes: &[u8], partial: bool, cancel: &AtomicBool) -> (Vec<Record>, Vec<String>) {
    match dialect {
        Dialect::Fish => fish(bytes, partial, cancel),
        Dialect::Zsh => zsh(bytes, partial, cancel),
        Dialect::Bash => bash(bytes, partial, cancel),
    }
}

fn fish_unescape(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            at += 1;
            match bytes.get(at)? {
                b'n' => result.push(b'\n'),
                b'\\' => result.push(b'\\'),
                _ => return None,
            }
        } else {
            result.push(bytes[at]);
        }
        at += 1;
    }
    Some(result)
}

fn fish(bytes: &[u8], partial: bool, cancel: &AtomicBool) -> (Vec<Record>, Vec<String>) {
    let mut result = Records::default();
    let mut notes = Vec::new();
    let mut current: Option<Record> = None;
    let mut skipping = partial;
    let mut invalid = 0usize;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !line.ends_with(b"\n") {
            current = None;
            notes.push("An interrupted final history line was skipped.".into());
            break;
        }
        let line = &line[..line.len() - 1];
        if skipping {
            skipping = false;
            continue;
        }
        if line.starts_with(b"- cmd: ") {
            if let Some(record) = current.take() {
                result.push(record)
            }
            if let Some(command) = fish_unescape(&line[7..]) {
                current = Some(Record { command, ..Record::default() })
            } else {
                invalid += 1
            }
        } else if let Some(record) = &mut current {
            if let Some(value) = line.strip_prefix(b"  when: ") {
                record.timestamp = std::str::from_utf8(value).ok().and_then(|value| value.parse().ok());
            } else if line.starts_with(b"  ") {
                let key = line[2..].split(|byte| *byte == b':').next().unwrap_or_default();
                if !key.is_empty()
                    && key.len() <= 64
                    && key.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    let key = String::from_utf8_lossy(key).into_owned();
                    if !record.fields.contains(&key) {
                        record.fields.push(key)
                    }
                } else if line.starts_with(b"  paths:") && !record.fields.iter().any(|field| field == "paths") {
                    record.fields.push("paths".into())
                }
            } else {
                invalid += 1;
                current = None;
            }
        }
    }
    if let Some(record) = current {
        result.push(record)
    }
    if invalid > 0 {
        notes.push(format!(
            "Skipped {invalid} malformed fish declarations or unsupported escapes; no shell evaluated them."
        ));
    }
    if result.values.is_empty() && notes.is_empty() && !bytes.is_empty() {
        notes.push("Unrecognized fish history format; native YAML-like command records are required.".into());
    }
    (result.finish(&mut notes), notes)
}

fn unmeta(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == 0x83 {
            at += 1;
            result.push(*bytes.get(at)? ^ 32)
        } else {
            result.push(bytes[at])
        }
        at += 1;
    }
    Some(result)
}

fn zsh(bytes: &[u8], partial: bool, cancel: &AtomicBool) -> (Vec<Record>, Vec<String>) {
    let mut result = Records::default();
    let mut notes = Vec::new();
    let mut command = Vec::new();
    let mut skipping = partial;
    let mut malformed = false;
    let mut invalid = false;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !line.ends_with(b"\n") {
            notes.push("An interrupted final history record was skipped.".into());
            break;
        }
        let line = &line[..line.len() - 1];
        if line.ends_with(b"\\") {
            if command.len() <= MAX_COMMAND_BYTES {
                command.extend_from_slice(&line[..line.len() - 1]);
                command.push(b'\n');
            }
            continue;
        }
        if command.len() <= MAX_COMMAND_BYTES {
            command.extend_from_slice(line);
        }
        if skipping {
            skipping = false;
            command.clear();
            continue;
        }
        // Zsh writes a padding space after a terminal backslash (and its
        // following spaces), so it won't be mistaken for a continued newline.
        let trimmed = command.iter().rposition(|byte| *byte != b' ');
        if command.ends_with(b" ") && trimmed.is_some_and(|at| command[at] == b'\\') {
            command.pop();
        }
        if let Some(mut decoded) = unmeta(&command) {
            let mut record = Record::default();
            if decoded.starts_with(b": ") {
                let header = decoded.iter().position(|byte| *byte == b';');
                if let Some(end) = header {
                    if let Some((timestamp, duration)) =
                        std::str::from_utf8(&decoded[2..end]).ok().and_then(|text| text.split_once(':'))
                    {
                        record.timestamp = timestamp.parse().ok();
                        record.duration = duration.parse().ok();
                        if record.timestamp.is_some() && record.duration.is_some() {
                            decoded.drain(..=end);
                        } else {
                            command.clear();
                            malformed = true;
                            continue;
                        }
                    } else {
                        command.clear();
                        malformed = true;
                        continue;
                    }
                } else {
                    command.clear();
                    malformed = true;
                    continue;
                }
            } else if decoded.starts_with(b"\\:") {
                decoded.remove(0);
            }
            record.command = decoded;
            result.push(record);
        } else {
            invalid = true;
        }
        command.clear();
    }
    if !command.is_empty() {
        notes.push("An unfinished Zsh continuation was skipped.".into());
    }
    if malformed {
        notes.push("Malformed extended Zsh records were skipped.".into());
    }
    if invalid {
        notes.push("Invalid Zsh metafied records were skipped.".into());
    }
    (result.finish(&mut notes), notes)
}

fn bash(bytes: &[u8], partial: bool, cancel: &AtomicBool) -> (Vec<Record>, Vec<String>) {
    let mut result = Records::default();
    let mut notes = Vec::new();
    let mut current: Option<Record> = None;
    let mut complete = true;
    let mut ambiguous = false;
    let mut skipping = partial;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !line.ends_with(b"\n") {
            complete = false;
            break;
        }
        if skipping {
            skipping = false;
            continue;
        }
        let text = &line[..line.len() - 1];
        let timestamp = text
            .strip_prefix(b"#")
            .filter(|value| !value.is_empty() && value.iter().all(u8::is_ascii_digit))
            .and_then(|value| std::str::from_utf8(value).ok())
            .and_then(|value| value.parse::<i64>().ok());
        if let Some(timestamp) = timestamp {
            if let Some(mut record) = current.take() {
                record.command.pop();
                result.push(record)
            }
            current = Some(Record { timestamp: Some(timestamp), ..Record::default() });
        } else if text.starts_with(b"#") && text.get(1).is_some_and(u8::is_ascii_digit) {
            current = None;
            ambiguous = true;
        } else if let Some(record) = &mut current {
            if record.command.len() <= MAX_COMMAND_BYTES {
                record.command.extend_from_slice(line);
            }
        } else if !text.is_empty() {
            ambiguous = true;
        }
    }
    if let Some(mut record) = current.filter(|_| complete) {
        record.command.pop();
        result.push(record)
    }
    if !complete {
        notes.push("An interrupted final Bash record was skipped.".into());
    }
    if ambiguous {
        notes.push("Untimestamped/ambiguous Bash text was skipped. Supported Bash recall requires timestamp-delimited history; its plain format cannot reliably delimit multiline commands.".into());
    }
    (result.finish(&mut notes), notes)
}
