//! Drawing the interface, ported from view.go.

use chrono::{DateTime, Datelike, Local, Utc};
use runui_core::golang::duration::SECOND;
use runui_project as project;
use runui_runs::{Run, State};
use runui_tea::ansi;
use runui_tea::bubbles::key::Binding;
use runui_tea::style::{self, CENTER, LEFT, TOP};

use crate::rows::{Item, Kind, View};
use crate::styles::*;
use crate::{App, Area, Focus, Geometry, PANEL_RUNS, RUNS_AGENTS, RUNS_EVERYWHERE};

/// PORTRAIT_BELOW is how wide the window must be for the sidebar beside
/// the pane: narrower, the panel with the focus goes on top of it.
const PORTRAIT_BELOW: i32 = 80;

impl App {
    /// layout shares the window out: the sidebar a third of it, the pane
    /// the rest, the status and help lines under them.
    pub(crate) fn layout(&mut self) {
        self.fh = (self.h - 2).max(3);
        self.portrait = self.w < PORTRAIT_BELOW;
        if self.full {
            (self.sw, self.side_h) = (0, 0);
            (self.pw, self.ph) = ((self.w - 2).max(10), (self.fh - 2).max(1));
        } else if self.portrait {
            (self.sw, self.side_h) = (self.w, (self.fh / 3).max(3));
            (self.pw, self.ph) = ((self.w - 2).max(10), (self.fh - self.side_h - 2).max(1));
        } else {
            (self.sw, self.side_h) = ((self.w / 3).clamp(30, 56), self.fh);
            (self.pw, self.ph) = ((self.w - self.sw - 2).max(10), (self.fh - 2).max(1));
        }
        self.help.set_width(self.w);
        let width = (self.pw - 14).max(20);
        if let Some(f) = self.form.as_mut() {
            for input in f.inputs() {
                input.set_width(width);
            }
        }
    }

    /// draw is the frame: the sidebar's panels beside the pane, or over it
    /// when the window is narrow, then the status line and the help bar.
    pub(crate) fn draw(&mut self) -> String {
        if self.w == 0 {
            return String::new();
        }
        let lines = self.pane_lines();
        let pane = self.pane_box(&lines);
        self.geom = Geometry::default();
        let frame = if self.full {
            self.geom.pane = Area { x: 0, y: 0, w: self.pw + 2, h: self.ph + 2 };
            pane
        } else if self.portrait {
            let side = self.sidebar(self.side_h);
            self.geom.pane = Area { x: 0, y: self.side_h, w: self.pw + 2, h: self.ph + 2 };
            style::join_vertical(LEFT, &[&side, &pane])
        } else {
            let side = self.sidebar(self.side_h);
            self.geom.pane = Area { x: self.sw, y: 0, w: self.pw + 2, h: self.ph + 2 };
            style::join_horizontal(TOP, &[&side, &pane])
        };
        let status = ansi::truncate(&self.status_line(), self.w, "…");
        let help = ansi::truncate(&self.help.view(&self.help_keys()), self.w, "…");
        style::join_vertical(LEFT, &[&frame, &status, &help])
    }

    // The sidebar

    /// heights shares the sidebar's lines among the panels: each as many as
    /// its rows need while they fit, else a share each, the one with the
    /// focus taking two, a panel needing less than its share giving the
    /// rest back. Too short for that, the panels without the focus fold to
    /// their title line.
    pub(crate) fn heights(&self, total: i32) -> Vec<i32> {
        let n = self.panels.len();
        let mut out = vec![0; n];
        if total < 3 * n as i32 {
            out.fill(1);
            out[self.cur] = (total - (n as i32 - 1)).max(3);
            return out;
        }
        let mut need = vec![0; n];
        let mut sum = 0;
        for (i, p) in self.panels.iter().enumerate() {
            let v = &p.views[p.cur];
            need[i] = ((v.shown.len() as i32).max(1) + 2 + v.filter_lines() + note_lines(v)).max(3);
            sum += need[i];
        }
        if sum <= total {
            out.copy_from_slice(&need);
            out[self.cur] += total - sum;
            return out;
        }
        let weight = |i: usize| if i == self.cur { 2 } else { 1 };
        let mut left = total;
        let mut flex: Vec<bool> = vec![true; n];
        let mut changed = true;
        while changed {
            changed = false;
            let weights: i32 = (0..n).filter(|&i| flex[i]).map(weight).sum();
            // [Go] a map's order, which varies; here the panels' own
            for i in (0..n).filter(|&i| flex[i]) {
                if need[i] <= left * weight(i) / weights {
                    (out[i], left, changed) = (need[i], left - need[i], true);
                    flex[i] = false;
                    break;
                }
            }
        }
        let weights: i32 = (0..n).filter(|&i| flex[i]).map(weight).sum();
        let mut given = 0;
        for i in (0..n).filter(|&i| flex[i]) {
            out[i] = left * weight(i) / weights;
            given += out[i];
        }
        if flex[self.cur] {
            out[self.cur] += left - given;
        } else if let Some(i) = (0..n).find(|&i| flex[i]) {
            out[i] += left - given;
        }
        out
    }

    /// sidebar is the panels stacked, height lines high: every one a box,
    /// or the one with the focus alone when the window is narrow. Where
    /// each box lands is kept for the mouse.
    fn sidebar(&mut self, height: i32) -> String {
        let mut lines = Vec::new();
        self.geom.panels = vec![Area::default(); self.panels.len()];
        if self.portrait {
            lines = self.panel_box(self.cur, self.sw, height);
            self.geom.panels[self.cur] = Area { x: 0, y: 0, w: self.sw, h: height };
        } else {
            let mut y = 0;
            for (i, h) in self.heights(height).into_iter().enumerate() {
                lines.extend(self.panel_box(i, self.sw, h));
                self.geom.panels[i] = Area { x: 0, y, w: self.sw, h };
                y += h;
            }
        }
        while (lines.len() as i32) < height {
            lines.push(" ".repeat(self.sw.max(0) as usize));
        }
        lines.truncate(height.max(0) as usize);
        lines.join("\n")
    }

    /// panel_box draws a panel, width wide and height high: its title line
    /// with its digit and tabs, its rows with the cursor's picked out, and
    /// its bottom line saying where the cursor is. One line high, it's the
    /// title line alone.
    fn panel_box(&mut self, i: usize, width: i32, height: i32) -> Vec<String> {
        let focused = i == self.cur && self.focus == Focus::Side && self.form.is_none();
        let line = if focused { S_ACCENT } else { S_LINE };
        let inner = (width - 2).max(1);
        let title = self.panel_title(i, inner, focused);
        let mut out = vec![
            line.render("╭")
                + &title
                + &line.render(&("─".repeat((inner - ansi::string_width(&title)).max(0) as usize) + "╮")),
        ];
        if height <= 1 {
            return out;
        }
        for r in self.panel_rows(i, inner, height - 2, focused) {
            out.push(line.render("│") + &pad(&r, inner) + &line.render("│"));
        }
        while (out.len() as i32) < height - 1 {
            out.push(line.render("│") + &" ".repeat(inner as usize) + &line.render("│"));
        }
        let p = &self.panels[i];
        let v = &p.views[p.cur];
        let mut foot = String::new();
        let n = v.shown.len();
        if n > 0 && v.at >= 0 {
            foot = format!("{} of {n}", v.at + 1);
        }
        if i == PANEL_RUNS {
            let live = self.live_count(v);
            if live > 0 {
                foot = format!("{live} running · {foot}");
            }
        }
        let foot = ansi::truncate(&foot, (inner - 2).max(0), "");
        let bottom = line
            .render(&("╰".to_string() + &"─".repeat((inner - ansi::string_width(&foot) - 1).max(0) as usize)))
            + &S_DIM.render(&foot)
            + &line.render("─╯");
        out.truncate((height - 1) as usize);
        out.push(bottom);
        out
    }

    fn live_count(&self, v: &View) -> usize {
        v.all.iter().filter(|it| self.run_of(&it.run.id).is_some_and(Run::live)).count()
    }

    /// panel_title is a panel's title line: its digit and its tabs, the one
    /// showing picked out.
    fn panel_title(&self, i: usize, width: i32, focused: bool) -> String {
        let p = &self.panels[i];
        let line = if focused { S_ACCENT } else { S_LINE };
        let tabs: Vec<String> = p
            .views
            .iter()
            .enumerate()
            .map(|(j, v)| {
                if j == p.cur && focused {
                    S_ACCENT.bold(true).render(&v.name)
                } else if j == p.cur {
                    S_TITLE.render(&v.name)
                } else {
                    S_DIM.render(&v.name)
                }
            })
            .collect();
        let title = line.render(&format!("─[{}]─", i + 1)) + &tabs.join(&S_DIM.render(" - "));
        ansi::truncate(&title, width.max(1), "…")
    }

    /// panel_rows are a view's rows that fit, the cursor's among them with a
    /// row of context, picked out in the accent while the panel has the
    /// focus and in grey when it hasn't; a problem reading it and the
    /// filter first.
    fn panel_rows(&mut self, i: usize, width: i32, height: i32, focused: bool) -> Vec<String> {
        // the tabs with runs from every folder: Everywhere, and Agents
        let everywhere = i == PANEL_RUNS && self.panels[i].cur >= RUNS_EVERYWHERE;
        let agents = i == PANEL_RUNS && self.panels[i].cur == RUNS_AGENTS;
        let p = &mut self.panels[i];
        let v = &mut p.views[p.cur];
        let mut out = Vec::new();
        if !v.note.is_empty() {
            out.push(S_BAD.render(&ansi::truncate(&format!("✗ {}", v.note), width, "…")));
        }
        if v.filter_lines() > 0 {
            let cursor = if v.typing { "▏" } else { "" };
            out.push(S_ACCENT.render(&ansi::truncate(&format!("/{}{cursor}", v.filter), width, "…")));
        }
        let height = height - out.len() as i32;
        if height <= 0 {
            return out;
        }
        if v.shown.is_empty() {
            let empty =
                if v.filter.is_empty() { v.empty.clone() } else { "nothing matches; esc clears the filter".into() };
            out.push(S_DIM.render(&ansi::truncate(&empty, width, "…")));
            return out;
        }
        let at = v.at;
        if height <= 2 {
            v.top = at;
        } else {
            if at < v.top + 1 {
                v.top = (at - 1).max(0);
            }
            if at >= v.top + height - 1 {
                v.top = at - height + 2;
            }
        }
        v.top = v.top.min(v.shown.len() as i32 - height).max(0);
        let top = v.top;
        let rows: Vec<Item> = v.shown.iter().skip(top as usize).take(height as usize).cloned().collect();
        for (n, it) in rows.iter().enumerate() {
            let mut r = ansi::truncate(&self.row(everywhere, agents, it), width, "…");
            if top + n as i32 == at {
                let style = if focused { S_ROW_ACTIVE } else { S_ROW_IDLE };
                r = style.render(&pad(&ansi::strip(&r), width));
            }
            out.push(r);
        }
        out
    }

    /// row is an item as its panel lists it: the state of its last run, a ★
    /// for a favorite, its name, and what it is, dimmed; in the Agents tab,
    /// a run's agent too.
    fn row(&self, everywhere: bool, agents: bool, it: &Item) -> String {
        let mark = match self.row_run(it) {
            Some(r) => self.state_mark(r),
            None => " ".to_string(),
        };
        // the folder under the project's it runs in
        let mut place = if it.place.is_empty() { String::new() } else { S_DIM.render(&format!("{} › ", it.place)) };
        match it.kind {
            Kind::Favorite => {
                // $ for a command of your own
                let star = if it.source.is_empty() { S_ACCENT.render("$") } else { S_WARN.render("★") };
                let hint =
                    if it.name != it.command { format!("  {}", S_DIM.render(&it.command)) } else { String::new() };
                format!("{mark} {star} {place}{}{hint}", it.name)
            }
            Kind::Command => {
                let star = if self.cfg.favorites.has(&self.cfg.dir, &it.favorite()) {
                    S_WARN.render("★")
                } else {
                    " ".to_string()
                };
                let hint = if it.desc.is_empty() { String::new() } else { format!("  {}", S_DIM.render(&it.desc)) };
                format!("{mark} {star} {place}{}{hint}", it.name)
            }
            Kind::Run => {
                let r = self.run_of(&it.run.id).cloned().unwrap_or_default();
                let mut hint = human(r.took((self.cfg.now)()));
                if agents && !r.agent.is_empty() {
                    hint = format!("{hint} · {}", r.agent);
                }
                if everywhere {
                    // everywhere: its whole folder instead
                    place = String::new();
                    hint = format!("{hint} · {}", self.short(&r.dir));
                }
                format!("{mark} {place}{}  {}", it.name, S_DIM.render(&hint))
            }
        }
    }

    /// row_run is the run a row's mark is for: its own, or its command's
    /// last.
    fn row_run(&self, it: &Item) -> Option<&Run> {
        if it.kind == Kind::Run {
            return self.run_of(&it.run.id);
        }
        self.last_run(&it.dir, &it.command)
    }

    /// state_mark is a run's state in a glyph: ● running, ✓ ended well, ✗
    /// ended badly, ■ stopped, ◌ starting or stopping, ? lost.
    fn state_mark(&self, r: &Run) -> String {
        if self.stopping.contains(&r.id) && r.live() {
            return S_WARN.render("◌");
        }
        match r.state() {
            State::Starting => S_ACCENT.render("◌"),
            State::Running => S_GOOD.render("●"),
            State::Stopped => S_DIM.render("■"),
            State::Lost => S_WARN.render("?"),
            State::Exited if r.exit == Some(0) => S_DIM.render("✓"),
            State::Exited => S_BAD.render("✗"),
        }
    }

    /// state_line says in words where a run is.
    fn state_line(&self, r: &Run) -> String {
        let now = (self.cfg.now)();
        let took = human(r.took(now));
        let at = match r.ended {
            Some(ended) => format!(" · {}", when(ended, now)),
            None => String::new(),
        };
        if self.stopping.contains(&r.id) && r.live() {
            return S_WARN.render("◌ stopping…");
        }
        match r.state() {
            State::Starting => return S_ACCENT.render("◌ starting…"),
            State::Running => {
                return S_GOOD.render(&format!("● running for {took}"))
                    + &S_DIM.render(&format!(
                        " · pid {} · s stops it, R runs it again; it keeps running when runui quits",
                        r.pid
                    ));
            }
            State::Stopped => return S_DIM.render(&format!("■ stopped after {took}{at}")),
            State::Lost => {
                return S_WARN.render(
                    "? lost track of it: its supervisor ended without saying how it went (killed, or the machine restarted)",
                ) + &S_DIM.render(" · D forgets it");
            }
            State::Exited => {}
        }
        if !r.error.is_empty() && r.pid == 0 {
            S_BAD.render(&format!("✗ couldn't start: {}", r.error))
        } else if r.exit == Some(0) {
            S_GOOD.render("✓ exited 0") + &S_DIM.render(&format!(" after {took}{at}"))
        } else if !r.signal.is_empty() {
            S_BAD.render(&format!("✗ ended by a signal, {}", r.signal)) + &S_DIM.render(&format!(" after {took}{at}"))
        } else if let Some(code) = r.exit {
            S_BAD.render(&format!("✗ exited {code}")) + &S_DIM.render(&format!(" after {took}{at}"))
        } else {
            S_DIM.render(&format!("ended{at}"))
        }
    }

    // The pane

    /// pane_box is the pane in its box, the title line naming what it
    /// shows, drawn in the accent while it has the focus.
    fn pane_box(&self, body: &[String]) -> String {
        let focused = self.focus == Focus::Pane || self.form.is_some() || self.confirm.is_some() || self.keys_shown;
        let (line, title_style) = if focused { (S_ACCENT, S_ACCENT.bold(true)) } else { (S_LINE, S_DIM) };
        let title = ansi::truncate(&self.pane_title(), (self.pw - 2).max(1), "…");
        let mut out = vec![
            line.render("╭─")
                + &title_style.render(&title)
                + &line.render(&("─".repeat((self.pw - 1 - ansi::string_width(&title)).max(0) as usize) + "╮")),
        ];
        for l in body {
            out.push(line.render("│") + &pad(l, self.pw) + &line.render("│"));
        }
        out.push(line.render(&("╰".to_string() + &"─".repeat(self.pw.max(0) as usize) + "╯")));
        out.join("\n")
    }

    /// pane_title names what the pane shows.
    fn pane_title(&self) -> String {
        if self.keys_shown {
            return "Keys".into();
        }
        match &self.form {
            Some(f) if f.old.is_some() => return "Edit the favorite".into(),
            Some(_) => return "A command of your own".into(),
            None => {}
        }
        let Some(it) = self.selected() else { return "runui".into() };
        if self.pane_run().is_some() {
            return format!("{} › output", it.name);
        }
        it.name
    }

    /// pane_lines are the pane's lines, as many as it's high, each cut to
    /// its width, so nothing pushes the sidebar or the help about.
    fn pane_lines(&mut self) -> Vec<String> {
        let mut lines;
        if self.keys_shown {
            lines = self.keys_lines();
        } else if let Some(c) = &self.confirm {
            let content = format!("{}\n\n{}", c.prompt, S_DIM.render("y or enter: yes · n or esc: no"));
            let box_ = S_BOX.width((self.pw - 4).min(80)).render(&content);
            lines = style::place(self.pw, self.ph, CENTER, CENTER, &box_).split('\n').map(str::to_string).collect();
        } else if self.form.is_some() {
            lines = self.form_lines();
        } else {
            lines = self.header();
            let h = self.out_height();
            if let Some(out) = &self.out {
                let n = out.len() as i32;
                if self.follow {
                    self.out_top = (n - h).max(0);
                }
                self.out_top = self.out_top.min((n - h).max(0)).max(0);
                if out.is_empty() {
                    lines.push(S_DIM.render("nothing printed yet"));
                }
                lines.extend(out.range(self.out_top as usize, (self.out_top + h) as usize));
            }
        }
        while (lines.len() as i32) < self.ph {
            lines.push(String::new());
        }
        lines.truncate(self.ph.max(0) as usize);
        lines.iter().map(|l| ansi::truncate(l, self.pw, "…")).collect()
    }

    /// out_height is how many lines of output the pane shows under its
    /// header.
    pub(crate) fn out_height(&self) -> i32 {
        (self.ph - self.header().len() as i32).max(1)
    }

    /// header is the top of the pane: what the row runs, where and how it
    /// went, over a rule when its output follows.
    fn header(&self) -> Vec<String> {
        let Some(it) = self.selected() else { return self.overview() };
        let mut name = S_TITLE.render(&it.name);
        if !it.place.is_empty() && it.kind != Kind::Run {
            name = S_DIM.render(&format!("{} › ", it.place)) + &name;
        }
        if self.cfg.favorites.has(&self.cfg.dir, &it.favorite()) {
            name += &S_WARN.render(" ★");
        }
        let mut lines = vec![name, S_ACCENT.render(&format!("$ {}", it.command)), S_DIM.render(&self.origin(&it))];
        if !it.desc.is_empty() && it.desc != it.command {
            lines.push(S_DIM.render(&it.desc));
        }
        let Some(r) = self.pane_run() else {
            let hint = if it.kind == Kind::Favorite {
                format!("enter runs it, in {}; e edits it, D removes it", self.short(&it.dir))
            } else {
                format!("enter runs it, in {}; m keeps it in the favorites", self.short(&it.dir))
            };
            lines.push(String::new());
            lines.push(S_DIM.render(&hint));
            return lines;
        };
        lines.push(String::new());
        lines.push(self.state_line(&r));
        if it.kind != Kind::Run && r.started < (self.cfg.now)() - chrono::TimeDelta::seconds(1) && !r.live() {
            let last = lines.len() - 1;
            lines[last] += &S_DIM.render(" · enter runs it again");
        }
        lines.push(S_LINE.render(&"─".repeat(self.pw.max(0) as usize)));
        lines
    }

    /// origin says where a row comes from, and the folder it runs in.
    fn origin(&self, it: &Item) -> String {
        let place = format!(" · in {}", self.short(&it.dir));
        if it.kind == Kind::Run && !it.run.agent.is_empty() {
            format!("started {} by {}{place}", when(it.run.started, (self.cfg.now)()), it.run.agent)
        } else if it.kind == Kind::Run {
            format!("started {}{place}", when(it.run.started, (self.cfg.now)()))
        } else if it.source.is_empty() {
            format!("a command of your own{place}")
        } else if !it.file.is_empty() {
            format!("from {}{place}", it.file)
        } else {
            format!("from {}{place}", project::label(&it.source))
        }
    }

    /// overview is the pane with no row under the cursor: the folder and
    /// its project files.
    fn overview(&self) -> Vec<String> {
        let mut lines = vec![S_TITLE.render(&format!("runui in {}", self.short(&self.cfg.dir))), String::new()];
        if self.files.is_empty() {
            lines.push(S_DIM.render(
                "No Taskfile, package.json, Makefile, compose file or pom.xml here, or in the folders under it.",
            ));
        }
        let width = self.files.iter().map(|f| f.rel_path().len()).max().unwrap_or(0);
        for f in &self.files {
            let what = match f.source {
                project::Source::Taskfile => "tasks",
                project::Source::Npm => "scripts",
                project::Source::Make => "targets",
                project::Source::Compose => "commands",
                project::Source::Maven => "builds",
            };
            let mut line = format!("{:<width$}  {} {what}", f.rel_path(), f.commands.len());
            if let Some(err) = &f.err {
                line += &S_BAD.render(&format!(" ✗ {err}"));
            }
            lines.push(line);
        }
        lines.push(String::new());
        lines
            .push(S_DIM.render("a adds a command of your own; tab and 1-3 go from panel to panel; ? shows every key."));
        lines
    }

    /// form_lines are the form: a name and a command.
    fn form_lines(&self) -> Vec<String> {
        let Some(f) = &self.form else { return Vec::new() };
        let label = |i: usize, s: &str| if f.at == i { S_ACCENT.bold(true).render(s) } else { S_DIM.render(s) };
        let mut lines = vec![
            S_HEADING.render(&self.pane_title()),
            S_DIM.render(&format!(
                "It runs with sh in {}, as you'd type it there, and it's kept in this folder's favorites.",
                self.short(&self.cfg.dir)
            )),
            String::new(),
            label(0, "Name     ") + "  " + &f.name.view(),
            label(1, "Command  ") + "  " + &f.command.view(),
            String::new(),
            S_DIM.render("tab: the other field · enter: save · esc: cancel"),
        ];
        if !f.err.is_empty() {
            lines.push(String::new());
            lines.push(S_BAD.render(&format!("✗ {}", f.err)));
        }
        lines
    }

    /// keys_lines are every key, explained.
    fn keys_lines(&self) -> Vec<String> {
        let k = &self.keys;
        let group = |title: &str, rows: &[(String, &str)]| {
            let mut out = vec![S_HEADING.render(title)];
            for (keys, what) in rows {
                out.push(format!("  {} {what}", S_ACCENT.render(&format!("{keys:<18}"))));
            }
            out.push(String::new());
            out
        };
        let h = |b: &Binding| b.help().key.clone();
        // the Agents tab, there once an agent has started something
        let tabs = if self.panels[PANEL_RUNS].views.len() > RUNS_AGENTS {
            "a panel's tabs: Taskfile, package.json, Makefile, Compose, Maven; here, everywhere and agents"
        } else {
            "a panel's tabs: Taskfile, package.json, Makefile, Compose, Maven; here and everywhere"
        };
        let mut lines = group(
            "Moving about",
            &[
                (format!("{} {}", h(&k.next_panel), h(&k.prev_panel)), "the next and the previous panel"),
                (h(&k.panels), "a panel: 1 Favorites, 2 the project's commands, 3 Runs"),
                (h(&k.tabs), tabs),
                ("↑/k ↓/j g G".into(), "along its rows"),
                (h(&k.filter), "filter its rows by what you type; esc clears it"),
                (
                    format!("{}, enter on a run", h(&k.focus_pane)),
                    "the output takes the keys, to scroll it; esc goes back",
                ),
                ("pgup/pgdn J/K".into(), "scroll the output from the panel; G there follows it again"),
                (h(&k.full), "the output gets the whole window, and back"),
            ],
        );
        lines.extend(group(
            "Commands",
            &[
                (h(&k.run), "run it, in its folder: the output shows on the right as it's printed"),
                (h(&k.stop), "stop it: its process group gets SIGTERM, and SIGKILL if it's there 5 s later"),
                (h(&k.restart), "run it again, stopping it first"),
                (h(&k.star), "keep it in this folder's favorites (★), or take it out"),
                (h(&k.add), "add a command of your own to the favorites"),
                (h(&k.edit), "edit a favorite: its name and its command"),
                (h(&k.remove), "take a favorite out, or forget a run that has ended and its output"),
                (format!("{} {}", h(&k.move_up), h(&k.move_down)), "move a favorite up or down"),
                (h(&k.yank), "copy the command"),
                (h(&k.reload), "read the project's files again (runui notices an edit on its own too)"),
            ],
        ));
        lines.extend(group(
            "Leaving",
            &[(h(&k.quit), "quit: what's running keeps running, and the next runui shows it, its output and all")],
        ));
        lines.push(S_DIM.render("any key closes this"));
        lines
    }

    // The lines under the frame

    fn status_line(&self) -> String {
        if !self.note.is_empty() && self.note_bad {
            return S_BAD.render(&format!("✗ {}", self.note));
        }
        if !self.note.is_empty() {
            return S_GOOD.render(&format!("✓ {}", self.note));
        }
        let live = self.runs.iter().filter(|r| self.here(r) && r.live()).count();
        let dir = self.short(&self.cfg.dir);
        match live {
            0 => S_DIM.render(&dir),
            1 => S_DIM.render(&format!("{dir} · 1 command running here; q leaves it running")),
            n => S_DIM.render(&format!("{dir} · {n} commands running here; q leaves them running")),
        }
    }

    /// help_keys are the keys the help bar shows for what has the focus.
    fn help_keys(&self) -> KeyHelp {
        let k = &self.keys;
        let bind = |name: &str, desc: &str| b(name, desc, &[name]);
        if self.form.is_some() {
            return KeyHelp::short(vec![bind("tab", "the other field"), bind("enter", "save"), bind("esc", "cancel")]);
        }
        if self.confirm.is_some() {
            return KeyHelp::short(vec![k.yes.clone(), k.no.clone()]);
        }
        if self.keys_shown {
            return KeyHelp::short(vec![bind("any key", "close")]);
        }
        if self.shown().typing {
            return KeyHelp::short(vec![bind("enter", "keep the filter"), bind("esc", "clear it")]);
        }
        if self.focus == Focus::Pane {
            return KeyHelp::short(vec![
                bind("↑/↓", "scroll"),
                bind("pgup/pgdn", "a page"),
                bind("G", "follow"),
                k.stop.clone(),
                k.restart.clone(),
                bind("esc", "back"),
                k.help.clone(),
                k.quit.clone(),
            ]);
        }
        let short = match self.selected() {
            None => vec![k.add.clone(), k.next_panel.clone(), k.panels.clone(), k.help.clone(), k.quit.clone()],
            Some(it) if it.kind == Kind::Run => vec![
                bind("enter", "its output"),
                k.stop.clone(),
                k.restart.clone(),
                bind("D", "forget it"),
                k.star.clone(),
                k.filter.clone(),
                k.help.clone(),
                k.quit.clone(),
            ],
            Some(it) if it.kind == Kind::Favorite => vec![
                k.run.clone(),
                k.stop.clone(),
                k.restart.clone(),
                k.edit.clone(),
                k.remove.clone(),
                bind("< >", "move"),
                k.add.clone(),
                k.help.clone(),
                k.quit.clone(),
            ],
            Some(_) => vec![
                k.run.clone(),
                k.star.clone(),
                k.stop.clone(),
                k.restart.clone(),
                k.add.clone(),
                k.filter.clone(),
                k.next_panel.clone(),
                k.help.clone(),
                k.quit.clone(),
            ],
        };
        KeyHelp::short(short)
    }

    // Words

    /// short is a folder with the home folder written ~.
    pub(crate) fn short(&self, dir: &str) -> String {
        let home = &self.cfg.home;
        if !home.is_empty() && (dir == home || dir.starts_with(&format!("{home}/"))) {
            return format!("~{}", &dir[home.len()..]);
        }
        dir.to_string()
    }
}

fn note_lines(v: &View) -> i32 {
    i32::from(!v.note.is_empty())
}

/// human is a duration in words: 4s, 3m 2s, 2h 5m, 3d 4h.
pub(crate) fn human(d: i64) -> String {
    let s = d.max(0) / SECOND;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else if s < 24 * 3600 {
        format!("{}h {}m", s / 3600, s / 60 % 60)
    } else {
        format!("{}d {}h", s / 3600 / 24, s / 3600 % 24)
    }
}

/// when is a time as the day it's in calls for: 14:03 today, Oct 3 14:03
/// before.
pub(crate) fn when(t: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let (t, now) = (t.with_timezone(&Local), now.with_timezone(&Local));
    if t.year() == now.year() && t.ordinal() == now.ordinal() {
        return t.format("%H:%M").to_string();
    }
    t.format("%b %-d %H:%M").to_string()
}

/// pad fills a line out to width with spaces.
pub(crate) fn pad(s: &str, width: i32) -> String {
    let w = ansi::string_width(s);
    if w < width {
        return s.to_string() + &" ".repeat((width - w) as usize);
    }
    ansi::truncate(s, width, "")
}
