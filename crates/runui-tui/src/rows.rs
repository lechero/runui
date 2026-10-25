//! A panel's rows, and the tabs that hold them: ported from rows.go.

use runui_core::golang::to_lower;
use runui_runs::Run;

/// Kind is what a row is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    /// a saved command
    #[default]
    Favorite,
    /// one of the project's tasks, scripts or targets
    Command,
    /// a command runui started
    Run,
}

/// Item is a row in a panel.
#[derive(Clone, Debug, Default)]
pub(crate) struct Item {
    pub(crate) kind: Kind,
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) desc: String,
    /// task, npm, make, compose or maven; "" for a command of your own
    pub(crate) source: String,
    /// where it runs
    pub(crate) dir: String,
    /// that folder from the project's, with slashes: "" for the project's own
    pub(crate) rel: String,
    /// what the row puts before its name: rel, and a compose file's name
    /// when it isn't the usual one
    pub(crate) place: String,
    /// the project file it's from, from the project's folder
    pub(crate) file: String,
    pub(crate) run: Run,
}

impl Item {
    /// key names a row across a refresh, to keep the cursor on it.
    pub(crate) fn key(&self) -> String {
        match self.kind {
            Kind::Run => format!("run {}", self.run.id),
            Kind::Favorite => format!("favorite {}\x00{}", self.rel, self.command),
            Kind::Command => format!("command {}\x00{}", self.dir, self.command),
        }
    }

    /// matches says the row has the words typed in the filter, in its name,
    /// command, description or folder: the one under the project's, or a
    /// run's whole path.
    pub(crate) fn matches(&self, filter: &str) -> bool {
        let place = if self.kind == Kind::Run { &self.dir } else { &self.place };
        let text = to_lower(&format!("{} {} {} {place}", self.name, self.command, self.desc));
        to_lower(filter).split_whitespace().all(|word| text.contains(word))
    }

    /// named says the row's name, with its folder, has every word of the
    /// filter.
    pub(crate) fn named(&self, filter: &str) -> bool {
        let text = to_lower(&format!("{} {}", self.place, self.name));
        to_lower(filter).split_whitespace().all(|word| text.contains(word))
    }
}

/// View is a panel's tab: its rows, the ones the filter lets through, and
/// where the cursor is.
#[derive(Clone, Debug, Default)]
pub(crate) struct View {
    pub(crate) name: String,
    pub(crate) all: Vec<Item>,
    pub(crate) shown: Vec<Item>,
    /// the cursor, in shown; -1 when there's nothing
    pub(crate) at: i32,
    /// the first row drawn
    pub(crate) top: i32,
    pub(crate) filter: String,
    /// the row the cursor was on before the filter
    pub(crate) before: String,
    /// the filter's being typed
    pub(crate) typing: bool,
    /// what the view says with no rows
    pub(crate) empty: String,
    /// a problem reading it, said above its rows
    pub(crate) note: String,
}

impl View {
    pub(crate) fn new(name: &str, empty: &str) -> View {
        View { name: name.to_string(), empty: empty.to_string(), ..Default::default() }
    }

    /// set gives the view its rows, the cursor kept on the row it was on.
    pub(crate) fn set(&mut self, items: Vec<Item>) {
        let keep = self.current().map(|it| it.key()).unwrap_or_default();
        self.all = items;
        self.apply(&keep);
    }

    /// set_filter filters the rows again, the cursor on the best match
    /// while there's a filter, and back on the row it was on once it's
    /// cleared.
    pub(crate) fn set_filter(&mut self, f: &str) {
        if self.filter.is_empty()
            && let Some(it) = self.current()
        {
            self.before = it.key();
        }
        self.filter = f.to_string();
        if f.is_empty() {
            let before = self.before.clone();
            self.apply(&before);
            return;
        }
        self.at = 0;
        self.apply("");
    }

    /// apply filters the rows: the ones whose name (and folder) has what's
    /// typed first, then the ones that have it only in their command or
    /// description, each in their order.
    fn apply(&mut self, keep: &str) {
        let (mut named, mut rest) = (Vec::new(), Vec::new());
        for it in &self.all {
            if !it.matches(&self.filter) {
                continue;
            }
            if it.named(&self.filter) {
                named.push(it.clone());
            } else {
                rest.push(it.clone());
            }
        }
        named.extend(rest);
        self.shown = named;
        if !self.pick(keep) {
            self.at = self.at.min(self.shown.len() as i32 - 1).max(0);
        }
        if self.shown.is_empty() {
            self.at = -1;
        }
    }

    /// pick puts the cursor on the row with the key, and says it's there.
    pub(crate) fn pick(&mut self, key: &str) -> bool {
        if key.is_empty() {
            return false;
        }
        match self.shown.iter().position(|it| it.key() == key) {
            Some(i) => {
                self.at = i as i32;
                true
            }
            None => false,
        }
    }

    pub(crate) fn current(&self) -> Option<&Item> {
        if self.at < 0 {
            return None;
        }
        self.shown.get(self.at as usize)
    }

    /// move_by moves the cursor by d rows, stopping at the ends (Go's
    /// `move`).
    pub(crate) fn move_by(&mut self, d: i32) {
        if self.shown.is_empty() {
            return;
        }
        self.at = (self.at + d).min(self.shown.len() as i32 - 1).max(0);
    }

    /// filter_lines is how many lines the filter takes above the rows.
    pub(crate) fn filter_lines(&self) -> i32 {
        i32::from(self.typing || !self.filter.is_empty())
    }
}
