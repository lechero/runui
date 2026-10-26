//! Four passive workers index explicit registrations. Requests return current
//! partial results immediately; obsolete generations cannot restore removed
//! projects. No native tools or shell history are inspected here.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use runui_project::Discovery;

use crate::registry::Project;

pub type Discover = fn(&str, &str, Arc<AtomicBool>) -> Discovery;
const WORKERS: usize = 4;
const CHECK_EVERY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug)]
pub struct Indexed {
    pub project: Project,
    pub discovery: Option<Arc<Discovery>>,
    pub branch: Option<String>,
    pub error: Option<String>,
    pub pending: bool,
}

#[derive(Clone, Default)]
pub struct Index {
    owner: Arc<Owner>,
}

#[derive(Default)]
struct Owner {
    shared: Arc<(Mutex<State>, Condvar)>,
    started: AtomicBool,
}

#[derive(Default)]
struct State {
    shutdown: bool,
    generation: u64,
    projects: BTreeMap<String, Project>,
    cached: BTreeMap<String, Cached>,
    queue: VecDeque<Work>,
    busy: HashSet<String>,
    cancel: Arc<AtomicBool>,
}

struct Cached {
    value: Indexed,
    checked: Instant,
}
struct Work {
    project: Project,
    generation: u64,
    previous: Option<Arc<Discovery>>,
    cancel: Arc<AtomicBool>,
    discover: Discover,
}

impl Drop for Owner {
    fn drop(&mut self) {
        let (mutex, changed) = &*self.shared;
        let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        state.shutdown = true;
        state.cancel.store(true, Ordering::Relaxed);
        state.queue.clear();
        changed.notify_all();
    }
}

impl Index {
    /// Schedule metadata checks or missing snapshots, retaining usable cached
    /// results. Membership changes cancel all work from the earlier generation.
    pub fn request(&self, projects: &[Project]) -> Vec<Indexed> {
        self.request_with(projects, runui_project::discover_cancellable)
    }

    pub fn request_with(&self, projects: &[Project], discover: Discover) -> Vec<Indexed> {
        let (mutex, changed) = &*self.owner.shared;
        let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        let projects: BTreeMap<_, _> = projects
            .iter()
            .take(crate::registry::MAX_PROJECTS)
            .map(|project| (project.id.clone(), project.clone()))
            .collect();
        if state.projects != projects {
            state.cancel.store(true, Ordering::Relaxed);
            state.cancel = Arc::new(AtomicBool::new(false));
            state.generation += 1;
            state.queue.clear();
            state.busy.clear();
            state.cached.retain(|id, _| projects.contains_key(id));
            state.projects = projects;
        }
        let now = Instant::now();
        for project in state.projects.values().cloned().collect::<Vec<_>>() {
            if state.busy.contains(&project.id)
                || state.cached.get(&project.id).is_some_and(|cache| now.duration_since(cache.checked) < CHECK_EVERY)
            {
                continue;
            }
            let work = Work {
                previous: state.cached.get(&project.id).and_then(|cache| cache.value.discovery.clone()),
                project: project.clone(),
                generation: state.generation,
                cancel: state.cancel.clone(),
                discover,
            };
            state.busy.insert(project.id);
            state.queue.push_back(work);
        }
        changed.notify_all();
        let result = view(&state);
        drop(state);
        if result.iter().any(|project| project.pending) && !self.owner.started.swap(true, Ordering::Relaxed) {
            for _ in 0..WORKERS {
                let shared = self.owner.shared.clone();
                std::thread::spawn(move || worker(shared));
            }
        }
        result
    }

    /// Used by CLI/MCP to collect a bounded first page. A slow checkout leaves
    /// a pending diagnostic while other workers' ready results remain usable.
    pub fn wait(&self, timeout: Duration) -> Vec<Indexed> {
        let (mutex, changed) = &*self.owner.shared;
        let state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        let (state, _) = changed
            .wait_timeout_while(state, timeout, |state| !state.busy.is_empty())
            .unwrap_or_else(|error| error.into_inner());
        view(&state)
    }

    pub fn invalidate(&self) {
        let (mutex, _) = &*self.owner.shared;
        let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        for cached in state.cached.values_mut() {
            cached.checked = Instant::now() - CHECK_EVERY;
        }
    }

    pub fn cancel_pending(&self) {
        let (mutex, changed) = &*self.owner.shared;
        let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        state.cancel.store(true, Ordering::Relaxed);
        state.cancel = Arc::new(AtomicBool::new(false));
        state.generation += 1;
        state.queue.clear();
        state.busy.clear();
        changed.notify_all();
    }
}

fn view(state: &State) -> Vec<Indexed> {
    let mut result: Vec<_> = state
        .projects
        .values()
        .map(|project| {
            let mut value = state.cached.get(&project.id).map(|cached| cached.value.clone()).unwrap_or_else(|| {
                Indexed { project: project.clone(), discovery: None, branch: None, error: None, pending: true }
            });
            value.project = project.clone();
            value.pending = state.busy.contains(&project.id);
            value
        })
        .collect();
    result.sort_by(|a, b| a.project.directory.cmp(&b.project.directory));
    result
}

fn worker(shared: Arc<(Mutex<State>, Condvar)>) {
    let (mutex, changed) = &*shared;
    loop {
        let work = {
            let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
            while state.queue.is_empty() && !state.shutdown {
                state = changed.wait(state).unwrap_or_else(|error| error.into_inner());
            }
            if state.shutdown {
                return;
            }
            state.queue.pop_front().expect("the queue was checked")
        };
        let value =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inspect(&work))).unwrap_or_else(|_| Indexed {
                project: work.project.clone(),
                discovery: None,
                branch: None,
                error: Some("passive discovery worker failed".into()),
                pending: false,
            });
        let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        if state.generation == work.generation && !work.cancel.load(Ordering::Relaxed) {
            state.busy.remove(&work.project.id);
            state.cached.insert(work.project.id, Cached { value, checked: Instant::now() });
        }
        changed.notify_all();
    }
}

fn inspect(work: &Work) -> Indexed {
    let mut value =
        Indexed { project: work.project.clone(), discovery: None, branch: None, error: None, pending: false };
    if let Err(error) = work.project.check_directory() {
        value.error = Some(error.to_string());
        return value;
    }
    value.branch = branch(&work.project.directory);
    value.discovery = Some(
        if let Some(previous) = &work.previous
            && !previous.changed()
        {
            previous.clone()
        } else {
            Arc::new((work.discover)(&work.project.directory, &work.project.directory, work.cancel.clone()))
        },
    );
    value
}

pub fn branch(directory: &str) -> Option<String> {
    let git = Path::new(directory).join(".git");
    let head = if git.is_dir() {
        git.join("HEAD")
    } else {
        let text = small_file(&git)?;
        let path = text.trim().strip_prefix("gitdir: ")?;
        Path::new(directory).join(path).join("HEAD")
    };
    let text = small_file(&head)?;
    let text = text.trim();
    if let Some(name) = text.strip_prefix("ref: refs/heads/") {
        Some(name.into())
    } else if text.len() >= 12 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(format!("detached {}", &text[..12]))
    } else {
        None
    }
}

fn small_file(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    fs::File::open(path).ok()?.take(4097).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests;
