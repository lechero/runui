//! Small local JSON files share one locking, recovery and atomic-write policy.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use fs2::FileExt;
use serde::{Serialize, de::DeserializeOwned};
use tempfile::NamedTempFile;

pub(crate) trait State: Default + Serialize + DeserializeOwned {
    fn version(&self) -> u32;
    fn recovered(&mut self, path: PathBuf);
}

#[derive(Clone, Debug)]
pub(crate) struct JsonFile {
    file: PathBuf,
    limit: u64,
    label: &'static str,
}

impl JsonFile {
    pub(crate) fn new(file: impl Into<PathBuf>, limit: u64, label: &'static str) -> Self {
        Self { file: file.into(), limit, label }
    }

    pub(crate) fn file(&self) -> &Path {
        &self.file
    }

    fn read(&self) -> std::io::Result<Vec<u8>> {
        let file = fs::File::open(&self.file)?;
        let mut bytes = Vec::new();
        file.take(self.limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{} exceeds the {} byte limit", self.label, self.limit),
            ));
        }
        Ok(bytes)
    }

    fn decode<T: State>(&self, bytes: &[u8]) -> Result<T> {
        let data: T = serde_json::from_slice(bytes).with_context(|| format!("invalid {}", self.label))?;
        if data.version() != 1 {
            return Err(anyhow!("unsupported {} version {}", self.label, data.version()));
        }
        Ok(data)
    }

    pub(crate) fn load<T: State>(&self) -> Result<T> {
        match self.read() {
            Ok(bytes) => self.decode(&bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
            Err(error) => Err(error.into()),
        }
        .with_context(|| format!("reading {}", self.file.display()))
    }

    pub(crate) fn change<T: State, R>(&self, apply: impl FnOnce(&mut T) -> Result<R>) -> Result<R> {
        let parent = self.file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.file.with_extension("lock"))?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match FileExt::try_lock_exclusive(&lock) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => return Err(error).with_context(|| format!("locking {}", self.label)),
            }
        }
        let mut data = match self.read() {
            Ok(bytes) => match self.decode::<T>(&bytes) {
                Ok(data) => data,
                Err(error) => {
                    let value = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
                    if value
                        .as_ref()
                        .and_then(|v| v.get("version"))
                        .is_some_and(|version| *version != serde_json::json!(1))
                    {
                        return Err(error);
                    }
                    let prefix = format!("{}.corrupt-", self.file.file_stem().unwrap_or_default().to_string_lossy());
                    let mut backup = tempfile::Builder::new().prefix(&prefix).suffix(".json").tempfile_in(parent)?;
                    backup.write_all(&bytes)?;
                    backup.as_file().sync_all()?;
                    let (_, path) = backup.keep().with_context(|| format!("preserving damaged {}", self.label))?;
                    let mut data = T::default();
                    data.recovered(path);
                    data
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => T::default(),
            Err(error) => return Err(error.into()),
        };
        let result = apply(&mut data)?;
        let mut temporary = NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temporary, &data)?;
        temporary.write_all(b"\n")?;
        if temporary.as_file().metadata()?.len() > self.limit {
            return Err(anyhow!("{} exceeds the {} byte limit", self.label, self.limit));
        }
        temporary.as_file().sync_all()?;
        temporary.persist(&self.file).with_context(|| format!("writing {}", self.file.display()))?;
        Ok(result)
    }
}
