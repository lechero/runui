//! Explicit checkout registrations. Reading the registry never discovers new
//! projects, starts tools, or changes favorites, processes or their output.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::state::{JsonFile, State};
use crate::{canonical, digest};

pub const MAX_PROJECTS: usize = 64;
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub directory: String,
    pub added: DateTime<Utc>,
}

impl Project {
    pub fn check_directory(&self) -> Result<()> {
        let path = Path::new(&self.directory);
        anyhow::ensure!(path.is_dir(), "registered checkout {} is unavailable", self.directory);
        let physical = path.canonicalize().with_context(|| format!("finding {}", self.directory))?;
        anyhow::ensure!(
            physical.to_str() == Some(self.directory.as_str()),
            "registered checkout {} now resolves elsewhere; register its new location explicitly",
            self.directory
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Data {
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovered_from: Option<PathBuf>,
}

impl Default for Data {
    fn default() -> Self {
        Self { version: 1, projects: Vec::new(), recovered_from: None }
    }
}

impl State for Data {
    fn version(&self) -> u32 {
        self.version
    }
    fn recovered(&mut self, path: PathBuf) {
        self.recovered_from = Some(path);
    }
}

impl Data {
    fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.projects.len() <= MAX_PROJECTS, "registry exceeds the {MAX_PROJECTS}-project limit");
        let mut identities = HashSet::new();
        for project in &self.projects {
            anyhow::ensure!(
                Path::new(&project.directory).is_absolute() && !project.directory.contains('\0'),
                "invalid registered checkout directory"
            );
            anyhow::ensure!(
                project.id == project_id(&project.directory),
                "registered checkout identity does not match its directory"
            );
            anyhow::ensure!(identities.insert(&project.id), "duplicate registered checkout {}", project.directory);
            valid_name(&project.name)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Store {
    file: JsonFile,
}

impl Store {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self { file: JsonFile::new(file, MAX_BYTES, "project registry") }
    }
    pub fn beside_favorites(file: &str) -> Self {
        Self::new(Path::new(file).parent().unwrap_or_else(|| Path::new(".")).join("projects-v1.json"))
    }
    pub fn file(&self) -> &Path {
        self.file.file()
    }
    pub fn load(&self) -> Result<Data> {
        let data: Data = self.file.load()?;
        data.validate().with_context(|| format!("reading {}", self.file().display()))?;
        Ok(data)
    }
    pub fn project(&self, id: &str) -> Result<Project> {
        self.load()?
            .projects
            .into_iter()
            .find(|project| project.id == id)
            .ok_or_else(|| anyhow!("project {id} is not registered"))
    }
    pub fn add(&self, directory: &str, home: &str, name: Option<&str>) -> Result<Project> {
        let directory = Path::new(directory).canonicalize().with_context(|| format!("finding checkout {directory}"))?;
        anyhow::ensure!(directory.is_dir(), "{} is not a directory", directory.display());
        let directory = directory.to_str().ok_or_else(|| anyhow!("checkout path is not UTF-8"))?;
        let directory = canonical(&runui_project::context_root(directory, home));
        let chosen_name = name.map(str::trim).filter(|name| !name.is_empty());
        let mut project = Project {
            id: project_id(&directory),
            name: chosen_name.map(str::to_owned).unwrap_or_else(|| {
                Path::new(&directory).file_name().unwrap_or_default().to_string_lossy().into_owned()
            }),
            directory,
            added: Utc::now(),
        };
        valid_name(&project.name)?;
        self.file.change(|data: &mut Data| {
            data.validate()?;
            if let Some(previous) = data.projects.iter_mut().find(|item| item.id == project.id) {
                project.added = previous.added;
                if chosen_name.is_none() {
                    project.name = previous.name.clone();
                }
                *previous = project.clone();
            } else {
                anyhow::ensure!(
                    data.projects.len() < MAX_PROJECTS,
                    "registry reached the {MAX_PROJECTS}-project limit; forget a checkout first"
                );
                data.projects.push(project.clone());
            }
            data.projects.sort_by(|a, b| a.directory.cmp(&b.directory));
            Ok(project)
        })
    }
    pub fn forget(&self, id_or_directory: &str) -> Result<bool> {
        let exact = runui_core::golang::filepath::abs(id_or_directory).ok();
        let exact = exact.map(|path| {
            let path = Path::new(&path);
            match (path.parent(), path.file_name()) {
                (Some(parent), Some(name)) => {
                    Path::new(&canonical(&parent.to_string_lossy())).join(name).to_string_lossy().into_owned()
                }
                _ => path.to_string_lossy().into_owned(),
            }
        });
        let directory =
            if id_or_directory.starts_with("project-v1-") { None } else { Some(canonical(id_or_directory)) };
        self.file.change(|data: &mut Data| {
            data.validate()?;
            let chosen = data
                .projects
                .iter()
                .find(|project| project.id == id_or_directory || Some(&project.directory) == exact.as_ref())
                .or_else(|| data.projects.iter().find(|project| Some(&project.directory) == directory.as_ref()))
                .map(|project| project.id.clone());
            if let Some(id) = chosen {
                data.projects.retain(|project| project.id != id);
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }
}

fn project_id(directory: &str) -> String {
    format!("project-v1-{}", digest(&[directory]))
}
fn valid_name(name: &str) -> Result<()> {
    anyhow::ensure!(
        !name.trim().is_empty() && name.chars().count() <= 160 && !name.chars().any(char::is_control),
        "project name must contain 1–160 printable characters"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
