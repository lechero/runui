//! Structured discovery and launch use the same catalog as the UI and MCP.

use std::collections::BTreeMap;
use std::io::Write;

use anyhow::anyhow;
use runui_catalog::{Inputs, Kind, Query, Scope};
use runui_core::golang::filepath;
use serde_json::json;

use super::{Failure, Workspace, mcp};

#[derive(Default)]
struct Options {
    directory: Option<String>,
    query: Option<String>,
    json: bool,
    fingerprint: Option<String>,
    mode: Option<String>,
    limit: Option<usize>,
    offset: usize,
    args: Vec<String>,
    variables: BTreeMap<String, String>,
    positional: Vec<String>,
    scope: Scope,
    project: Option<String>,
    timeout_seconds: Option<u32>,
    native: bool,
    cargo_global_aliases: bool,
}

fn parse(command: &str, args: &[String]) -> Result<Options, Failure> {
    let mut result = Options::default();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        index += 1;
        if arg == "--" {
            if command != "run" {
                return Err(Failure::Usage(format!("{command} does not accept launch arguments")));
            }
            result.args.extend_from_slice(&args[index..]);
            break;
        }
        if arg == "--native" && command == "inspect" {
            result.native = true;
            continue;
        }
        if arg == "--cargo-global-aliases" {
            result.cargo_global_aliases = true;
            continue;
        }
        if arg == "--json" {
            result.json = true;
            continue;
        }
        if arg == "--help" || arg == "-h" {
            return Err(Failure::Help);
        }
        if !arg.starts_with('-') {
            result.positional.push(arg.clone());
            continue;
        }
        let (name, inline) =
            arg.split_once('=').map_or((arg.as_str(), None), |(key, value)| (key, Some(value.to_string())));
        if !matches!(
            name,
            "-C" | "--C"
                | "--query"
                | "--fingerprint"
                | "--mode"
                | "--limit"
                | "--offset"
                | "--var"
                | "--scope"
                | "--project"
                | "--timeout"
        ) {
            return Err(Failure::Usage(format!("unknown catalog option {name}")));
        }
        let applicable = match name {
            "--query" | "--mode" | "--limit" | "--offset" => matches!(command, "commands" | "find"),
            "--fingerprint" | "--var" | "--timeout" => command == "run",
            "--project" => matches!(command, "inspect" | "run"),
            _ => true,
        };
        if !applicable {
            return Err(Failure::Usage(format!("{command} does not accept {name}")));
        }
        let value = if let Some(value) = inline {
            value
        } else {
            let value = args.get(index).cloned().ok_or_else(|| Failure::Usage(format!("{name} needs a value")))?;
            index += 1;
            value
        };
        match name {
            "-C" | "--C" => result.directory = Some(value),
            "--query" => result.query = Some(value),
            "--fingerprint" => result.fingerprint = Some(value),
            "--mode" => result.mode = Some(value),
            "--timeout" => {
                result.timeout_seconds = Some(
                    value
                        .parse()
                        .map_err(|_| Failure::Usage("--timeout needs seconds as a non-negative integer".into()))?,
                )
            }
            "--limit" => {
                let size = value.parse::<usize>().map_err(|_| Failure::Usage("--limit needs an integer".into()))?;
                if !(1..=500).contains(&size) {
                    return Err(Failure::Usage("--limit must be between 1 and 500".into()));
                }
                result.limit = Some(size);
            }
            "--offset" => {
                result.offset =
                    value.parse().map_err(|_| Failure::Usage("--offset needs a non-negative integer".into()))?
            }
            "--scope" if value == "project" => result.scope = Scope::Project,
            "--scope" if value == "workspace" => result.scope = Scope::Workspace,
            "--scope" if value == "registered" => result.scope = Scope::Registered,
            "--project" => result.project = Some(value),
            "--scope" => return Err(Failure::Usage(format!("scope {value:?} is not available yet"))),
            "--var" => {
                let (name, value) =
                    value.split_once('=').ok_or_else(|| Failure::Usage("--var needs NAME=VALUE".into()))?;
                result.variables.insert(name.into(), value.into());
            }
            _ => unreachable!(),
        }
    }
    if result.query.is_some() && !result.positional.is_empty() {
        return Err(Failure::Usage("provide either --query or positional search text".into()));
    }
    if matches!(command, "inspect" | "run") && result.scope == Scope::Registered && result.project.is_none() {
        return Err(Failure::Usage("registered inspect/run requires --project PROJECT_ID from discovery".into()));
    }
    if result.project.is_some() && result.scope != Scope::Registered {
        return Err(Failure::Usage("--project requires registered scope".into()));
    }
    Ok(result)
}

fn workspace(directory: Option<&str>) -> anyhow::Result<Workspace> {
    let directory = directory
        .filter(|directory| !directory.is_empty())
        .map(str::to_string)
        .unwrap_or(filepath::getwd().map_err(anyhow::Error::from)?);
    let absolute = filepath::abs(&directory).map_err(anyhow::Error::from)?;
    let home = filepath::user_home_dir().unwrap_or_default();
    let dir = runui_project::root(&absolute, &home);
    Ok(Workspace {
        repo: runui_project::context_root(&dir, &home),
        dir,
        home,
        invocation_dir: absolute,
        index: runui_catalog::index::Index::default(),
    })
}

pub(super) fn dispatch(
    cancel: &std::sync::atomic::AtomicBool,
    command: &str,
    args: &[String],
    output: &mut dyn Write,
) -> Result<(), Failure> {
    let options = parse(command, args)?;
    let workspace = workspace(options.directory.as_deref())?;
    let snapshot = if matches!(command, "inspect" | "run") {
        workspace.catalog_selection(options.scope, options.project.as_deref(), options.cargo_global_aliases)?
    } else {
        workspace.catalog_scoped(options.scope, options.cargo_global_aliases)?
    };
    match command {
        "commands" | "find" => {
            if !options.args.is_empty() || !options.variables.is_empty() || options.fingerprint.is_some() {
                return Err(Failure::Usage("search does not accept launch inputs".into()));
            }
            let text = options.query.unwrap_or_else(|| options.positional.join(" "));
            let mut query = Query::parse(&text).map_err(Failure::Usage)?;
            query.kinds = match options.mode.as_deref().unwrap_or("commands") {
                "commands" => {
                    query.commands();
                    query.kinds.clone()
                }
                "recent" => vec![Kind::Recent],
                "all" => Vec::new(),
                mode => return Err(Failure::Usage(format!("mode {mode:?} is not available yet"))),
            };
            let limit = options.limit.unwrap_or(200);
            query.limit = limit + 1;
            query.offset = options.offset;
            let hits = snapshot.search(&query);
            let more = hits.len() > limit;
            let entries: Vec<_> = hits.iter().take(limit).map(|hit| hit.entry).collect();
            if options.json {
                serde_json::to_writer_pretty(&mut *output,&json!({"schema_version":1,"project_directory":workspace.dir,"scope":options.scope,"cargo_global_aliases":options.cargo_global_aliases,"commands":entries,"diagnostics":snapshot.diagnostics,"complete":snapshot.complete(),"next_offset":if more {Some(options.offset+limit)} else {None}})).map_err(anyhow::Error::from)?;
                writeln!(output).map_err(anyhow::Error::from)?;
            } else {
                for entry in entries {
                    writeln!(
                        output,
                        "{}  {}  {}  {}  {}",
                        entry.id,
                        text_line(&entry.name),
                        text_line(&entry.source),
                        text_line(&entry.relative_source_directory()),
                        text_line(&entry.project_directory)
                    )
                    .map_err(anyhow::Error::from)?;
                }
                for diagnostic in &snapshot.diagnostics {
                    writeln!(
                        output,
                        "Discovery {}: {}: {}",
                        if diagnostic.kind == "notice" { "notice" } else { "incomplete" },
                        text_line(&diagnostic.path),
                        text_line(&diagnostic.message)
                    )
                    .map_err(anyhow::Error::from)?;
                }
            }
        }
        "inspect" | "run" => {
            if options.positional.len() != 1 {
                return Err(Failure::Usage(format!("{command} needs one command ID")));
            }
            let id = &options.positional[0];
            if command == "inspect" {
                if !options.args.is_empty() || !options.variables.is_empty() {
                    return Err(Failure::Usage("inspect does not accept launch inputs".into()));
                }
                let entry = snapshot.get(id).ok_or_else(|| anyhow!("command {id} does not exist in this scope"))?;
                let native = options.native.then(|| runui_catalog::inspection::check(entry, cancel));
                if options.json {
                    let mut value = serde_json::to_value(entry).map_err(anyhow::Error::from)?;
                    if let Some(report) = &native {
                        value["native_inspection"] = json!(report);
                    }
                    serde_json::to_writer_pretty(&mut *output, &value).map_err(anyhow::Error::from)?;
                    writeln!(output).map_err(anyhow::Error::from)?;
                } else {
                    writeln!(output,"{}\n{}\n$ {}\ncwd: {}\nsource: {}\nentry: {}\ndefinition: {}\nfingerprint: {}\navailability: {}",entry.id,text_line(&entry.name),text_line(&entry.command),text_line(&entry.directory),entry.source,text_line(entry.entry_file.as_deref().unwrap_or("saved command")),text_line(entry.defining_file.as_deref().unwrap_or("saved command")),entry.fingerprint,entry.availability.label()).map_err(anyhow::Error::from)?;
                    if let Some(report) = &native {
                        writeln!(
                            output,
                            "Native inspection: {}",
                            text_line(&serde_json::to_string(report).map_err(anyhow::Error::from)?)
                        )
                        .map_err(anyhow::Error::from)?;
                    }
                }
            } else {
                let inputs = Inputs {
                    args: options.args,
                    variables: options.variables,
                    timeout_seconds: options.timeout_seconds,
                };
                let prepared =
                    snapshot.prepare(id, options.fingerprint.as_deref(), &inputs).map_err(anyhow::Error::from)?;
                let mut store = workspace.runs()?;
                if !prepared.shell.is_empty() {
                    store.shell = prepared.shell.clone();
                }
                let (run, error) = store.start(&prepared.directory, &prepared.name, &prepared.command);
                if let Some(error) = error {
                    return Err(Failure::Err(error));
                }
                let history_error = runui_catalog::usage::Store::for_runs(&store.root)
                    .record(&run, Some(&prepared))
                    .err()
                    .map(|error| error.to_string());
                if options.json {
                    let mut data = mcp::run_data(&run);
                    data["definition_id"] = json!(prepared.id);
                    data["fingerprint"] = json!(prepared.fingerprint);
                    if let Some(error) = history_error {
                        data["history_error"] = json!(error);
                    }
                    serde_json::to_writer_pretty(&mut *output, &data).map_err(anyhow::Error::from)?;
                    writeln!(output).map_err(anyhow::Error::from)?;
                } else {
                    writeln!(output, "{}", run.id).map_err(anyhow::Error::from)?;
                    if let Some(error) = history_error {
                        writeln!(output, "Run started; usage history could not be saved: {}", text_line(&error))
                            .map_err(anyhow::Error::from)?;
                    }
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

pub(super) fn text_line(text: &str) -> String {
    text.chars()
        .flat_map(|ch| if ch.is_control() { ch.escape_default().collect::<Vec<_>>() } else { vec![ch] })
        .collect()
}
