//! Explicit, read-only shell recall. Nothing here is enrolled in Snapshot or
//! usage: callers must opt in and pass a source, and MCP never calls this module.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

mod formats;
#[cfg(test)]
mod tests;

pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 2000;
pub const MAX_COMMAND_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dialect {
    Fish,
    Zsh,
    Bash,
}

impl Dialect {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "fish" => Ok(Self::Fish),
            "zsh" => Ok(Self::Zsh),
            "bash" => Ok(Self::Bash),
            _ => Err("shell history supports fish, zsh or bash".into()),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fish => "fish",
            Self::Zsh => "zsh",
            Self::Bash => "bash",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Fish => Self::Zsh,
            Self::Zsh => Self::Bash,
            Self::Bash => Self::Fish,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub dialect: Dialect,
    /// None honors a disabled/private history session; no filesystem read occurs.
    pub path: Option<PathBuf>,
}

impl Source {
    /// Resolves names only. No file, shell startup, universal variable or history
    /// database is read. Nonexported session settings need an explicit path.
    pub fn current(dialect: Dialect, home: &str) -> Self {
        let path = match dialect {
            Dialect::Fish => {
                let name = std::env::var_os("fish_history").unwrap_or_else(|| "fish".into());
                if name.is_empty() {
                    None
                } else if let Some(name) =
                    name.to_str().filter(|name| !name.contains(['/', '\\']) && *name != "." && *name != "..")
                {
                    let data = std::env::var_os("XDG_DATA_HOME")
                        .filter(|value| !value.is_empty())
                        .map(PathBuf::from)
                        .unwrap_or_else(|| Path::new(home).join(".local/share"));
                    Some(data.join("fish").join(format!("{}_history", if name == "default" { "fish" } else { name })))
                } else {
                    None
                }
            }
            Dialect::Zsh | Dialect::Bash => match std::env::var_os("HISTFILE") {
                Some(value) if value.is_empty() => None,
                Some(value)
                    if std::env::var_os("SHELL").is_some_and(|shell| {
                        Path::new(&shell).file_name().is_some_and(|name| name == dialect.as_str())
                    }) =>
                {
                    Some(PathBuf::from(value))
                }
                _ => Some(Path::new(home).join(if dialect == Dialect::Zsh { ".zsh_history" } else { ".bash_history" })),
            },
        };
        Self { dialect, path }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub command: String,
    pub dialect: Dialect,
    pub timestamp: Option<DateTime<Utc>>,
    pub duration_seconds: Option<u64>,
    pub directory: Option<String>,
    pub exit_code: Option<i32>,
    pub source_file: String,
    /// Source metadata we don't interpret, including fish's required `paths`.
    pub metadata_fields: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    bytes: u64,
    modified: i64,
    nanos: i64,
}
impl Stamp {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes: metadata.len(),
            modified: metadata.mtime(),
            nanos: metadata.mtime_nsec(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Data {
    pub source: Source,
    pub entries: Vec<Entry>,
    pub diagnostics: Vec<String>,
    pub bytes_read: usize,
    stamp: Option<Stamp>,
}

impl Data {
    pub fn changed(&self) -> bool {
        self.source.path.as_ref().and_then(|path| fs::metadata(path).ok()).as_ref().map(Stamp::of) != self.stamp
    }

    /// Literal terms, not catalog facets or shell syntax. Record multiplicity
    /// is not frequency. Text quality precedes timestamp and stable file order.
    pub fn search(&self, text: &str, limit: usize, offset: usize) -> Vec<&Entry> {
        let terms: Vec<_> = text.split_whitespace().map(str::to_lowercase).collect();
        let mut hits = Vec::new();
        for (order, entry) in self.entries.iter().enumerate() {
            let command = entry.command.to_lowercase();
            let score = terms.iter().try_fold(0usize, |sum, term| {
                let quality = if command == *term {
                    600
                } else if command.starts_with(term) {
                    450
                } else if command.contains(term) {
                    350
                } else {
                    let mut chars = command.chars();
                    let mut gaps = 0usize;
                    for needle in term.chars() {
                        loop {
                            match chars.next() {
                                Some(ch) if ch == needle => break,
                                Some(_) => gaps += 1,
                                None => return None,
                            }
                        }
                    }
                    200usize.saturating_sub(gaps)
                };
                Some(sum + quality)
            });
            if let Some(score) = score {
                hits.push((score, order, entry));
            }
        }
        hits.sort_by(|a, b| {
            b.0.cmp(&a.0).then(b.2.timestamp.cmp(&a.2.timestamp)).then(a.1.cmp(&b.1)).then(a.2.id.cmp(&b.2.id))
        });
        hits.into_iter().skip(offset).take(limit.min(1000)).map(|(_, _, entry)| entry).collect()
    }
}

#[derive(Default)]
struct Record {
    command: Vec<u8>,
    timestamp: Option<i64>,
    duration: Option<u64>,
    fields: Vec<String>,
}

pub fn load(source: &Source, cancel: &AtomicBool) -> Data {
    load_with_limit(source, cancel, MAX_BYTES, MAX_ENTRIES)
}

fn load_with_limit(source: &Source, cancel: &AtomicBool, max_bytes: usize, max_entries: usize) -> Data {
    let mut data =
        Data { source: source.clone(), entries: Vec::new(), diagnostics: Vec::new(), bytes_read: 0, stamp: None };
    let Some(path) = &source.path else {
        data.diagnostics.push("This history source is disabled/private; no file was read.".into());
        return data;
    };
    data.stamp = fs::metadata(path).ok().as_ref().map(Stamp::of);
    let result = (|| -> anyhow::Result<()> {
        ensure!(!cancel.load(Ordering::Relaxed), "Shell history read cancelled.");
        let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
        let metadata = file.metadata()?;
        data.stamp = Some(Stamp::of(&metadata));
        ensure!(metadata.is_file(), "Shell history must be a regular file.");
        let start = metadata.len().saturating_sub(max_bytes as u64);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take(max_bytes as u64).read_to_end(&mut bytes)?;
        data.bytes_read = bytes.len();
        ensure!(!cancel.load(Ordering::Relaxed), "Shell history read cancelled.");
        if start > 0 {
            data.diagnostics
                .push(format!("Only the last {max_bytes} bytes were read; the first partial record was skipped."));
        }
        let (records, diagnostics) = formats::parse(source.dialect, &bytes, start > 0, cancel);
        data.diagnostics.extend(diagnostics);
        let mut seen = HashSet::new();
        let mut skipped = 0usize;
        for record in records.into_iter().rev() {
            ensure!(!cancel.load(Ordering::Relaxed), "Shell history read cancelled.");
            if record.command.is_empty() || record.command.len() > MAX_COMMAND_BYTES || record.command.contains(&0) {
                skipped += 1;
                continue;
            }
            let Ok(command) = String::from_utf8(record.command) else {
                skipped += 1;
                continue;
            };
            let id = format!("recall-v1-{}", crate::digest(&[source.dialect.as_str(), &command]));
            if !seen.insert(id.clone()) {
                continue;
            }
            if data.entries.len() >= max_entries {
                data.diagnostics.push(format!("Only the latest {max_entries} distinct records are shown; stored record counts are not execution frequency."));
                break;
            }
            data.entries.push(Entry {
                id,
                command,
                dialect: source.dialect,
                timestamp: record.timestamp.and_then(|value| DateTime::from_timestamp(value, 0)),
                duration_seconds: record.duration,
                directory: None,
                exit_code: None,
                source_file: path.to_string_lossy().into_owned(),
                metadata_fields: record.fields,
            });
        }
        if skipped > 0 {
            data.diagnostics.push(format!("Skipped {skipped} empty, oversized, NUL-containing or non-UTF-8 records; command text was not repaired."));
        }
        if data.changed() {
            data.diagnostics.push("History changed or rotated during reading; refresh to see the new snapshot.".into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        if cancel.load(Ordering::Relaxed) {
            data.entries.clear();
        }
        data.diagnostics.push(error.to_string());
    }
    data
}

#[derive(Clone, Debug, Serialize)]
pub struct Prepared {
    pub entry: Entry,
    pub directory: String,
    pub command: String,
    pub shell: Vec<String>,
}

/// Replay is deliberately separate from selection. A current record, explicit
/// directory and installed matching shell are required; nothing is started here.
pub fn prepare(
    source: &Source,
    id: &str,
    directory: &str,
    program: &str,
    cancel: &AtomicBool,
) -> anyhow::Result<Prepared> {
    ensure!(
        Path::new(directory).is_absolute() && Path::new(directory).is_dir(),
        "Choose an existing absolute working directory; history does not record its original cwd."
    );
    ensure!(
        Path::new(program).is_absolute()
            && fs::metadata(program).is_ok_and(|metadata| metadata.is_file() && metadata.mode() & 0o111 != 0),
        "The matching shell is not installed; use Copy or insert in a matching interactive shell."
    );
    ensure!(
        Path::new(program).file_name().is_some_and(|name| name == source.dialect.as_str()),
        "The selected executable does not match this history's shell dialect."
    );
    let data = load(source, cancel);
    ensure!(!cancel.load(Ordering::Relaxed), "Shell history preparation cancelled.");
    let entry = data.entries.into_iter().find(|entry| entry.id == id).ok_or_else(|| {
        anyhow!("The selected history record is no longer available; refresh before replaying or saving.")
    })?;
    Ok(Prepared {
        directory: crate::canonical(directory),
        command: entry.command.clone(),
        shell: vec![program.into(), "-c".into()],
        entry,
    })
}

impl Prepared {
    /// Existing raw favorites run through sh. An explicit native-shell wrapper
    /// preserves the promoted entry's dialect without changing stored formats.
    pub fn favorite_command(&self) -> String {
        format!("{} -c {}", crate::quote_arg(&self.shell[0]), crate::quote_arg(&self.command))
    }
}
