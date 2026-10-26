//! Each folder's saved commands: the project's tasks, scripts and targets
//! starred with m, its folders' too, and commands of your own, in one
//! file, ~/.config/runui/favorites.json. Ported from
//! internal/runui/favorites; the file is the Go build's, byte for byte, so
//! the two share it.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use runui_core::golang::{self, filepath, jsontext, opt_time};
use serde::Serialize;

mod decode;

/// Favorite is a saved command. It's read as Go's encoding/json reads it
/// (decode.rs): a field's name in any case, the last of a field given
/// twice, a null leaving it as it was.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Favorite {
    /// the task's, script's or target's, or one of your own
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// the command line, run with sh in the folder
    pub command: String,
    /// task, npm, make, compose or maven for one starred from the project's
    /// files; "" for one of your own
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// the folder it runs in, from the folder it's a favorite of, with
    /// slashes: apps/web; "" for that folder
    #[serde(skip_serializing_if = "String::is_empty")]
    pub dir: String,
    #[serde(with = "opt_time", skip_serializing_if = "Option::is_none")]
    pub added: Option<DateTime<Utc>>,
}

impl Favorite {
    /// same says two favorites are one: the same command in the same folder.
    pub fn same(&self, g: &Favorite) -> bool {
        self.command == g.command
            && filepath::clean(&format!("./{}", self.dir)) == filepath::clean(&format!("./{}", g.dir))
    }

    /// label is what the favorite is called: its name, else its command.
    pub fn label(&self) -> &str {
        if self.name.is_empty() { &self.command } else { &self.name }
    }

    /// own says the favorite is a command of your own rather than one of
    /// the project's.
    pub fn own(&self) -> bool {
        self.source.is_empty()
    }
}

/// Store is the favorites file. Every change reads the file again first,
/// so two runuis open at once don't undo each other's.
#[derive(Clone, Debug)]
pub struct Store {
    file: String,
    folders: BTreeMap<String, Vec<Favorite>>,
}

#[derive(Default, Serialize)]
struct FileData {
    folders: BTreeMap<String, Vec<Favorite>>,
}

/// default_file is ~/.config/runui/favorites.json, or under
/// $XDG_CONFIG_HOME when that's set.
pub fn default_file() -> Result<String> {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME")
        && !dir.is_empty()
    {
        return Ok(filepath::join(&[&dir, "runui", "favorites.json"]));
    }
    let home = filepath::user_home_dir()?;
    Ok(filepath::join(&[&home, ".config", "runui", "favorites.json"]))
}

impl Store {
    /// open reads the favorites file; one that isn't there yet is empty.
    pub fn open(file: &str) -> Result<Store> {
        let mut s = Store { file: file.to_string(), folders: BTreeMap::new() };
        s.reload()?;
        Ok(s)
    }

    /// file is where the favorites are kept.
    pub fn file(&self) -> &str {
        &self.file
    }

    /// for_dir is a folder's favorites in their order (Go's `For`).
    pub fn for_dir(&self, dir: &str) -> Vec<Favorite> {
        self.folders.get(&key(dir)).cloned().unwrap_or_default()
    }

    /// Project workspace search projects saved commands into absolute execution
    /// directories without changing their original folder keys or ordering.
    pub fn for_tree(&self, dir: &str) -> Vec<Favorite> {
        let physical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let root = physical(Path::new(dir));
        let mut out = Vec::new();
        for (folder, favorites) in &self.folders {
            if !physical(Path::new(folder)).starts_with(&root) {
                continue;
            }
            for favorite in favorites {
                let directory = physical(&Path::new(folder).join(&favorite.dir));
                if !directory.starts_with(&root) {
                    continue;
                }
                let mut projected = favorite.clone();
                projected.dir = directory.to_string_lossy().into_owned();
                out.push(projected);
            }
        }
        out
    }

    /// has says a folder has a command, run where f says, among its
    /// favorites.
    pub fn has(&self, dir: &str, f: &Favorite) -> bool {
        self.index(dir, f).is_some()
    }

    /// add puts a command at the end of a folder's favorites; the same one
    /// already there (same) is renamed instead, keeping its place.
    pub fn add(&mut self, dir: &str, mut f: Favorite) -> Result<()> {
        f.command = f.command.trim().to_string();
        f.name = f.name.trim().to_string();
        if f.command.is_empty() {
            return Err(anyhow!("a favorite needs a command"));
        }
        self.change(|s| {
            let k = key(dir);
            if let Some(i) = s.index(dir, &f) {
                if !f.name.is_empty() {
                    s.folders.get_mut(&k).expect("the folder found")[i].name = f.name.clone();
                }
                return;
            }
            if f.added.is_none() {
                f.added = Some(Utc::now());
            }
            s.folders.entry(k).or_default().push(f.clone());
        })
    }

    /// update changes the favorite old to f, in its place.
    pub fn update(&mut self, dir: &str, old: &Favorite, mut f: Favorite) -> Result<()> {
        f.command = f.command.trim().to_string();
        f.name = f.name.trim().to_string();
        if f.command.is_empty() {
            return Err(anyhow!("a favorite needs a command"));
        }
        let mut err = None;
        let cerr = self.change(|s| {
            let Some(i) = s.index(dir, old) else {
                err = Some(anyhow!("{} isn't a favorite here any more", old.command));
                return;
            };
            let was = s.folders[&key(dir)][i].clone();
            if f.dir.is_empty() {
                f.dir = was.dir.clone(); // it stays where it runs
            }
            if let Some(j) = s.index(dir, &f)
                && j != i
            {
                err = Some(anyhow!("{} is a favorite here already", f.command));
                return;
            }
            f.added = was.added;
            if f.command != was.command {
                f.source = String::new(); // edited, it's a command of your own now
            } else {
                f.source = was.source;
            }
            s.folders.get_mut(&key(dir)).expect("the folder found")[i] = f.clone();
        });
        join(err, cerr.err())
    }

    /// remove takes a favorite out of a folder's favorites.
    pub fn remove(&mut self, dir: &str, f: &Favorite) -> Result<()> {
        self.change(|s| {
            let k = key(dir);
            if let Some(list) = s.folders.get_mut(&k) {
                list.retain(|g| !f.same(g));
                if list.is_empty() {
                    s.folders.remove(&k);
                }
            }
        })
    }

    /// toggle stars a command, or unstars it when it's a favorite already;
    /// it says which.
    pub fn toggle(&mut self, dir: &str, f: Favorite) -> Result<bool> {
        self.reload()?;
        if self.has(dir, &f) {
            return self.remove(dir, &f).map(|_| false);
        }
        self.add(dir, f).map(|_| true)
    }

    /// move_by moves a folder's favorite by delta places: -1 up, 1 down
    /// (Go's `Move`).
    pub fn move_by(&mut self, dir: &str, f: &Favorite, delta: i32) -> Result<()> {
        self.change(|s| {
            let Some(i) = s.index(dir, f) else { return };
            let j = i as i64 + i64::from(delta);
            let Some(list) = s.folders.get_mut(&key(dir)) else { return };
            if j < 0 || j >= list.len() as i64 {
                return;
            }
            list.swap(i, j as usize);
        })
    }

    /// reload reads the file again, for changes another runui made.
    pub fn reload(&mut self) -> Result<()> {
        self.folders = BTreeMap::new();
        let data = match golang::read_file(Path::new(&self.file)) {
            Ok(data) => data,
            Err(e) if golang::is_not_exist(&e) => return Ok(()),
            Err(e) => return Err(e),
        };
        let d = unmarshal(&data).map_err(|e| anyhow!("{}: {e}", self.file))?;
        for (dir, favs) in d.folders {
            self.folders.insert(key(&dir), favs);
        }
        Ok(())
    }

    fn index(&self, dir: &str, f: &Favorite) -> Option<usize> {
        self.folders.get(&key(dir))?.iter().position(|g| f.same(g))
    }

    /// change applies a change to what's in the file now and writes it back.
    fn change(&mut self, apply: impl FnOnce(&mut Store)) -> Result<()> {
        self.reload()?;
        apply(self);
        self.save()
    }

    /// save writes the file whole, through a temporary file, so a reader
    /// never sees half of it.
    fn save(&self) -> Result<()> {
        let mut data = golang::marshal_indent(&FileData { folders: self.folders.clone() }, "", "  ")?;
        data.push(b'\n');
        let dir = filepath::dir(&self.file);
        golang::mkdir_all(Path::new(&dir), 0o755)?;
        let (mut tmp, name) = golang::create_temp(Path::new(&dir), ".favorites-*")?;
        if let Err(e) = tmp.write_all(&data) {
            drop(tmp);
            let _ = std::fs::remove_file(&name);
            return Err(golang::path_error("write", &name, e));
        }
        drop(tmp);
        golang::rename(&name, Path::new(&self.file))
    }
}

/// unmarshal reads the file the way Go's `json.Unmarshal` reads it into
/// its fileData: a syntax error, or a document that isn't an object, in
/// its words; a null a zero.
fn unmarshal(data: &[u8]) -> std::result::Result<FileData, String> {
    jsontext::check_valid(data).map_err(|e| e.to_string())?;
    let first = data.iter().copied().find(|c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r'));
    let kind = match first {
        // the words alone, without serde's place: Go's say where in theirs
        Some(b'{') => {
            return serde_json::from_slice(data).map_err(|e| {
                let msg = e.to_string();
                let place = format!(" at line {} column {}", e.line(), e.column());
                msg.strip_suffix(&place).map(str::to_string).unwrap_or(msg)
            });
        }
        Some(b'n') => return Ok(FileData::default()),
        Some(b'[') => "array",
        Some(b'"') => "string",
        Some(b't' | b'f') => "bool",
        _ => "number",
    };
    Err(format!("json: cannot unmarshal {kind} into Go value of type favorites.fileData"))
}

/// join is Go's errors.Join of two: either, or both a line each.
fn join(a: Option<anyhow::Error>, b: Option<anyhow::Error>) -> Result<()> {
    match (a, b) {
        (None, None) => Ok(()),
        (Some(e), None) | (None, Some(e)) => Err(e),
        (Some(a), Some(b)) => Err(anyhow!("{a}\n{b}")),
    }
}

/// key is how a folder is kept: its path, cleaned.
fn key(dir: &str) -> String {
    filepath::clean(dir)
}

#[cfg(test)]
mod tests;
