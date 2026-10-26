//! Versioned usage state. Files are supplied by callers; this module never
//! discovers HOME or reads a shell history. A separate lock file survives atomic
//! replacement of the data file, so concurrent frontends do not lose launches.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, TimeDelta, Utc};
use fs2::FileExt;
use runui_runs::Run;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::{Entry, Prepared, Recipe, canonical, digest, run_state};

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const RECENT_LIMIT: usize = 1000;
const COUNT_LIMIT: usize = 2000;
const RETENTION_DAYS: i64 = 180;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invocation {
    pub run_id: String,
    pub name: String,
    pub directory: String,
    pub command: String,
    pub recipe: Recipe,
    pub started: DateTime<Utc>,
    pub definition_id: Option<String>,
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    pub state: Option<String>,
    pub agent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Count {
    pub launches: u64,
    pub last_used: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Data {
    pub version: u32,
    #[serde(default)]
    pub recent: Vec<Invocation>,
    #[serde(default)]
    pub counts: BTreeMap<String, Count>,
    #[serde(default)]
    seen: BTreeMap<String, DateTime<Utc>>,
    /// A damaged file is preserved before a successful write repairs history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovered_from: Option<PathBuf>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            version: VERSION,
            recent: Vec::new(),
            counts: BTreeMap::new(),
            seen: BTreeMap::new(),
            recovered_from: None,
        }
    }
}

impl Data {
    pub(crate) fn decorate(&self, entry: &mut Entry) {
        let invocation = invocation_key(&entry.directory, &entry.command, recipe_shell(&entry.recipe));
        let definition = self.counts.get(&format!("definition:{}", entry.id));
        let raw = self.counts.get(&format!("unattributed:{invocation}")).filter(|count| count.launches > 0);
        if matches!(entry.kind, crate::Kind::Recent | crate::Kind::Run) {
            if let Some(count) = self.counts.get(&invocation) {
                entry.launches = count.launches;
                entry.last_used = Some(count.last_used);
            }
        } else {
            entry.launches =
                definition.map_or(0, |count| count.launches).saturating_add(raw.map_or(0, |count| count.launches));
            entry.last_used = definition.map(|count| count.last_used).max(raw.map(|count| count.last_used));
        }
    }

    fn count(&mut self, key: String, started: DateTime<Utc>) {
        let count = self.counts.entry(key).or_insert(Count { launches: 0, last_used: started });
        count.launches = count.launches.saturating_add(1);
        count.last_used = count.last_used.max(started);
    }

    fn record(&mut self, run: &Run, prepared: Option<&Prepared>, now: DateTime<Utc>) -> Result<()> {
        if run.id.is_empty() {
            return Err(anyhow!("a launch needs a run ID"));
        }
        let directory = canonical(&run.dir);
        if let Some(selection) = prepared
            && (canonical(&selection.directory) != directory
                || selection.command != run.command
                || (!selection.shell.is_empty() && selection.shell != run.shell))
        {
            return Err(anyhow!("the prepared invocation does not match the started run"));
        }
        let definition = prepared.and_then(|selection| {
            if let Some(run_id) = selection.id.strip_prefix("recent:") {
                self.recent
                    .iter()
                    .find(|invocation| invocation.run_id == run_id)
                    .and_then(|invocation| invocation.definition_id.clone())
            } else {
                Some(selection.id.clone())
            }
        });
        if self.seen.contains_key(&run.id) {
            // A frontend can enrich a raw launch after another caller recorded
            // it, but replaying the same notification never increments usage.
            let annotate =
                self.recent.iter().any(|invocation| invocation.run_id == run.id && invocation.definition_id.is_none())
                    && definition.is_some();
            if annotate && let Some(id) = &definition {
                self.count(format!("definition:{id}"), run.started);
                let raw = format!("unattributed:{}", invocation_key(&directory, &run.command, &run.shell));
                if let Some(count) = self.counts.get_mut(&raw) {
                    count.launches = count.launches.saturating_sub(1);
                }
                if self.counts.get(&raw).is_some_and(|count| count.launches == 0) {
                    self.counts.remove(&raw);
                }
            }
            if let Some(invocation) = self.recent.iter_mut().find(|invocation| invocation.run_id == run.id) {
                invocation.state = Some(observed_state(invocation.state.as_deref(), run));
                if annotate {
                    invocation.definition_id = definition;
                    invocation.source = prepared.map(|selection| selection.source.clone());
                    invocation.fingerprint = prepared
                        .filter(|selection| !selection.id.starts_with("recent:"))
                        .map(|selection| selection.fingerprint.clone());
                }
            }
            return Ok(());
        }
        // Import/reconciliation is not a launch backfill. Detailed history is
        // bounded; events older than its retention window are not new usage.
        if run.started < now - TimeDelta::days(RETENTION_DAYS) {
            return Ok(());
        }
        self.seen.insert(run.id.clone(), run.started);
        self.count(invocation_key(&directory, &run.command, &run.shell), run.started);
        if let Some(id) = &definition {
            self.count(format!("definition:{id}"), run.started);
        } else {
            self.count(format!("unattributed:{}", invocation_key(&directory, &run.command, &run.shell)), run.started);
        }
        self.recent.push(Invocation {
            run_id: run.id.clone(),
            name: run.label().into(),
            directory,
            command: run.command.clone(),
            recipe: Recipe::Shell { command: run.command.clone(), shell: run.shell.clone() },
            started: run.started,
            definition_id: definition,
            source: prepared.map(|selection| selection.source.clone()),
            fingerprint: prepared
                .filter(|selection| !selection.id.starts_with("recent:"))
                .map(|selection| selection.fingerprint.clone()),
            state: Some(observed_state(None, run)),
            agent: if run.agent.is_empty() { None } else { Some(run.agent.clone()) },
        });
        self.prune(now);
        Ok(())
    }

    fn prune(&mut self, now: DateTime<Utc>) {
        let cutoff = now - TimeDelta::days(RETENTION_DAYS);
        self.recent.retain(|invocation| invocation.started >= cutoff);
        self.recent.sort_by(|a, b| b.started.cmp(&a.started).then(b.run_id.cmp(&a.run_id)));
        self.recent.truncate(RECENT_LIMIT);
        self.seen.retain(|_, started| *started >= cutoff);
        if self.counts.len() > COUNT_LIMIT {
            let mut keys: Vec<_> = self.counts.iter().map(|(key, count)| (key.clone(), count.last_used)).collect();
            keys.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let keep: HashSet<_> = keys.into_iter().take(COUNT_LIMIT).map(|(key, _)| key).collect();
            self.counts.retain(|key, _| keep.contains(key));
        }
    }
}

fn observed_state(previous: Option<&str>, run: &Run) -> String {
    let state = if run.error.is_empty() { run_state(run) } else { "launch_error".into() };
    if matches!(previous, Some("completed" | "failed" | "stopped" | "launch_error"))
        && matches!(state.as_str(), "running" | "lost")
    {
        previous.expect("matched a known terminal state").into()
    } else {
        state
    }
}

fn recipe_shell(recipe: &Recipe) -> &[String] {
    match recipe {
        Recipe::Shell { shell, .. } | Recipe::Tool { shell, .. } => shell,
    }
}
fn invocation_key(directory: &str, command: &str, shell: &[String]) -> String {
    let shell = serde_json::to_string(shell).expect("shell arguments are strings");
    format!("invocation:{}", digest(&[directory, command, &shell]))
}

#[derive(Clone, Debug)]
pub struct Store {
    file: PathBuf,
}

impl Store {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }
    pub fn for_runs(root: &str) -> Self {
        Self::new(Path::new(root).join("catalog-v1.json"))
    }
    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn load(&self) -> Result<Data> {
        let bytes = match read_state(&self.file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Data::default()),
            Err(error) => return Err(error).with_context(|| format!("reading {}", self.file.display())),
        };
        decode(&bytes).with_context(|| format!("reading {}", self.file.display()))
    }

    pub fn record(&self, run: &Run, prepared: Option<&Prepared>) -> Result<()> {
        self.change(|data| data.record(run, prepared, Utc::now()))
    }

    /// Updating observed outcomes neither creates launches nor changes notices.
    pub fn reconcile(&self, runs: &[Run]) -> Result<()> {
        let data = self.load()?;
        let now = Utc::now();
        let cutoff = now - TimeDelta::days(RETENTION_DAYS);
        let changed = runs.iter().any(|run| run.started >= cutoff && !data.seen.contains_key(&run.id))
            || data.recent.iter().any(|invocation| {
                invocation.started < cutoff
                    || runs.iter().any(|run| {
                        run.id == invocation.run_id
                            && invocation.state.as_deref()
                                != Some(observed_state(invocation.state.as_deref(), run).as_str())
                    })
            })
            || data.seen.values().any(|started| *started < cutoff);
        if !changed {
            return Ok(());
        }
        self.change(|data| {
            // Persisted run metadata proves a launch even if a frontend exited
            // between creating the run and recording usage. Keep its original
            // timestamp; observing the same run is never another use.
            for run in runs {
                data.record(run, None, now)?;
            }
            data.prune(now);
            Ok(())
        })
    }

    pub fn forget(&self, run_id: &str) -> Result<()> {
        self.change(|data| {
            data.recent.retain(|invocation| invocation.run_id != run_id);
            Ok(())
        })
    }

    fn change(&self, apply: impl FnOnce(&mut Data) -> Result<()>) -> Result<()> {
        let parent = self.file.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        let lock_path = self.file.with_extension("lock");
        let lock =
            OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(&lock_path)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match FileExt::try_lock_exclusive(&lock) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => return Err(error).context("locking usage history"),
            }
        }
        let mut data = match read_state(&self.file) {
            Ok(bytes) => {
                // A future schema is never overwritten by an older binary.
                match decode(&bytes) {
                    Ok(data) => data,
                    Err(error) => {
                        let value = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
                        if value
                            .as_ref()
                            .and_then(|value| value["version"].as_u64())
                            .is_some_and(|version| version != u64::from(VERSION))
                        {
                            return Err(error);
                        }
                        let mut backup = tempfile::Builder::new()
                            .prefix("catalog-v1.corrupt-")
                            .suffix(".json")
                            .tempfile_in(parent)?;
                        backup.write_all(&bytes)?;
                        backup.as_file().sync_all()?;
                        let (_, path) = backup.keep().context("preserving damaged usage history")?;
                        Data { recovered_from: Some(path), ..Data::default() }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Data::default(),
            Err(error) => return Err(error.into()),
        };
        apply(&mut data)?;
        let mut temporary = NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temporary, &data)?;
        temporary.write_all(b"\n")?;
        if temporary.as_file().metadata()?.len() > MAX_BYTES {
            return Err(anyhow!("usage history exceeds the 16 MiB limit"));
        }
        temporary.as_file().sync_all()?;
        temporary.persist(&self.file).with_context(|| format!("writing {}", self.file.display()))?;
        Ok(())
    }
}

fn read_state(path: &Path) -> std::io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "usage history exceeds the 16 MiB limit"));
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<Data> {
    let data: Data = serde_json::from_slice(bytes).context("invalid usage history")?;
    if data.version != VERSION {
        return Err(anyhow!("unsupported usage history version {}", data.version));
    }
    Ok(data)
}

#[cfg(test)]
mod tests;
