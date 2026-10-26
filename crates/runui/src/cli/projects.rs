//! Explicit registry management. No command execution or automatic enrollment.

use std::io::Write;

use runui_catalog::{index, registry};
use serde_json::json;

use super::Failure;

pub(super) fn dispatch(args: &[String], output: &mut dyn Write) -> Result<(), Failure> {
    let mut positional = Vec::new();
    let mut json_output = false;
    let mut name = None;
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--json" => json_output = true,
            "--name" => {
                name = Some(arguments.next().cloned().ok_or_else(|| Failure::Usage("--name needs a value".into()))?)
            }
            "--" => {
                positional.extend(arguments.cloned());
                break;
            }
            "-h" | "--help" => return Err(Failure::Help),
            value if value.starts_with("--name=") => name = Some(value[7..].into()),
            value if value.starts_with('-') => return Err(Failure::Usage(format!("unknown projects option {value}"))),
            _ => positional.push(argument.clone()),
        }
    }
    if name.is_some() && positional.first().is_none_or(|action| action != "add") {
        return Err(Failure::Usage("--name applies only to projects add".into()));
    }
    let store = registry::Store::beside_favorites(&runui_favorites::default_file()?);
    let result = match positional.as_slice() {
        [] => {
            let data = store.load()?;
            let projects: Vec<_> = data
                .projects
                .iter()
                .map(|project| {
                    let mut value = serde_json::to_value(project).expect("project fields are serializable");
                    value["available"] = json!(project.check_directory().is_ok());
                    value["branch"] = json!(index::branch(&project.directory));
                    value
                })
                .collect();
            if !json_output {
                for project in &projects {
                    writeln!(
                        output,
                        "{}  {}  {}  {}",
                        project["id"].as_str().unwrap_or_default(),
                        safe(project["name"].as_str().unwrap_or_default()),
                        safe(project["directory"].as_str().unwrap_or_default()),
                        if project["available"] == true { "available" } else { "unavailable" }
                    )
                    .map_err(anyhow::Error::from)?;
                }
            }
            json!({"schema_version":1,"registry_file":store.file(),"projects":projects,"recovered_from":data.recovered_from})
        }
        [action, directory] if action == "add" => {
            let home = runui_core::golang::filepath::user_home_dir().unwrap_or_default();
            let project = store.add(directory, &home, name.as_deref())?;
            if !json_output {
                writeln!(output, "Registered {}: {}", safe(&project.name), safe(&project.directory))
                    .map_err(anyhow::Error::from)?;
            }
            json!({"schema_version":1,"project":project})
        }
        [action, identity] if action == "forget" => {
            let forgotten = store.forget(identity)?;
            if !json_output {
                writeln!(output, "{} {}", if forgotten { "Forgot" } else { "Not registered:" }, safe(identity))
                    .map_err(anyhow::Error::from)?;
            }
            json!({"schema_version":1,"forgotten":forgotten})
        }
        _ => return Err(Failure::Usage("projects accepts [add DIR [--name NAME]|forget ID_OR_DIR] [--json]".into())),
    };
    if json_output {
        serde_json::to_writer_pretty(&mut *output, &result).map_err(anyhow::Error::from)?;
        writeln!(output).map_err(anyhow::Error::from)?;
    }
    Ok(())
}

fn safe(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| if ch.is_control() { ch.escape_default().collect::<Vec<_>>() } else { vec![ch] })
        .collect()
}
