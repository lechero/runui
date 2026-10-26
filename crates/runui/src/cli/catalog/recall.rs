//! Explicit local recall has no catalog, usage or run-store side effects.
use super::{Failure, Options, text_line};
use runui_catalog::{Scope, recall};
use runui_core::golang::filepath;
use runui_tea::tea::Program;
use serde_json::json;
use std::io::{IsTerminal, Write};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) fn dispatch(cancel: &Arc<AtomicBool>, options: &Options, out: &mut dyn Write) -> Result<(), Failure> {
    if options.scope != Scope::Project || options.project.is_some() || options.cargo_global_aliases {
        return Err(Failure::Usage(
            "Shell history has unknown cwd; project/workspace/registered and Cargo scope options do not apply.".into(),
        ));
    }
    if options.directory.is_some() {
        return Err(Failure::Usage(
            "Shell recall has unknown cwd; -C does not filter it. Choose cwd explicitly when replaying in the TUI."
                .into(),
        ));
    }
    let dialect = options
        .shell
        .ok_or_else(|| Failure::Usage("Opt in with --shell fish|zsh|bash when using find --mode shell.".into()))?;
    if options.print0 && (!options.pick || options.json) {
        return Err(Failure::Usage("--print0 requires --pick and cannot be combined with --json.".into()));
    }
    if options.pick && (options.limit.is_some() || options.offset != 0) {
        return Err(Failure::Usage("Interactive selection does not accept pagination options.".into()));
    }
    let home = filepath::user_home_dir().unwrap_or_default();
    let mut source = recall::Source::current(dialect, &home);
    if let Some(path) = &options.history_file {
        if path.is_empty() {
            return Err(Failure::Usage("--history-file needs a non-empty explicit path.".into()));
        }
        source.path = Some(std::path::PathBuf::from(filepath::abs(path).map_err(anyhow::Error::from)?));
    }
    let query = options.query.clone().unwrap_or_else(|| options.positional.join(" "));
    if options.pick {
        if !std::io::stderr().is_terminal() {
            return Err(Failure::Usage(
                "Shell selection needs a terminal on stderr; use --json for noninteractive recall.".into(),
            ));
        }
        let directory = options.directory.clone().unwrap_or(filepath::getwd().map_err(anyhow::Error::from)?);
        let settings = runui_tui::HistorySettings { sources: vec![source], programs: Vec::new() };
        let picker = Program::new(runui_tui::ShellPicker::new(settings, &directory, &query))
            .with_stderr()
            .with_cancel(cancel.clone())
            .run()
            .map_err(|error| {
                if cancel.load(Ordering::Relaxed) {
                    Failure::Cancelled
                } else {
                    Failure::from(anyhow::Error::from(error))
                }
            })?;
        let Some(entry) = picker.selection() else { return Err(Failure::Cancelled) };
        if entry.dialect != dialect {
            return Err(Failure::Usage("Selection dialect differs from the requesting shell; copy it instead.".into()));
        }
        if options.print0 {
            out.write_all(entry.command.as_bytes()).map_err(anyhow::Error::from)?;
            out.write_all(&[0]).map_err(anyhow::Error::from)?;
        } else {
            serde_json::to_writer_pretty(
                &mut *out,
                &json!({"schema_version":1,"action":"insert","insertion_dialect":dialect,"selection":entry}),
            )
            .map_err(anyhow::Error::from)?;
            writeln!(out).map_err(anyhow::Error::from)?;
        }
        return Ok(());
    }
    let data = recall::load(&source, cancel);
    if cancel.load(Ordering::Relaxed) {
        return Err(Failure::Cancelled);
    }
    let limit = options.limit.unwrap_or(200);
    let rows = data.search(&query, limit + 1, options.offset);
    let more = rows.len() > limit;
    let records: Vec<_> = rows.into_iter().take(limit).collect();
    if options.json {
        serde_json::to_writer_pretty(&mut *out,&json!({"schema_version":1,"private":true,"dialect":dialect,"history":records,"diagnostics":data.diagnostics,"bytes_read":data.bytes_read,"next_offset":if more {Some(options.offset+limit)} else {None}})).map_err(anyhow::Error::from)?;
        writeln!(out).map_err(anyhow::Error::from)?;
    } else {
        for entry in records {
            writeln!(out, "{}  {}  cwd unknown  {}", entry.id, entry.dialect.as_str(), text_line(&entry.command))
                .map_err(anyhow::Error::from)?;
        }
        for diagnostic in data.diagnostics {
            writeln!(out, "History: {}", text_line(&diagnostic)).map_err(anyhow::Error::from)?;
        }
    }
    Ok(())
}
