use crate::{
    cli::{Command, ConfigCommand, ConnectionCommand},
    config_doctor,
    config_runtime::RuntimeConfig,
    render::{RenderFormat, TerminalEvent, render_event},
};
use std::{fs, path::PathBuf};

pub fn run(
    command: Command,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        Command::Config { command } => config(command, runtime, format),
        Command::Connection { command } => connection(command, runtime, format),
        Command::Ask { prompt, file } => ask(prompt, file, format),
        Command::Query { sql, file } => query(sql, file, format),
    }
}

fn config(
    command: ConfigCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        ConfigCommand::Init => {
            print!("{CONFIG_TEMPLATE}");
            Ok(0)
        }
        ConfigCommand::Doctor => {
            let message = config_doctor::summary(runtime);
            emit(TerminalEvent::Result { message }, format);
            Ok(0)
        }
        ConfigCommand::Show { .. } => {
            let value = runtime.resolved.redacted_diagnostics();
            let output = match format {
                RenderFormat::Text => serde_json::to_string_pretty(&value)?,
                _ => serde_json::to_string(&value)?,
            };
            println!("{output}");
            Ok(0)
        }
    }
}

fn connection(
    command: ConnectionCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        ConnectionCommand::List => {
            let names = runtime
                .connections
                .profiles
                .iter()
                .map(|(name, profile)| format!("{name} ({})", profile.dialect().as_str()))
                .collect::<Vec<_>>();
            emit(
                TerminalEvent::Result {
                    message: if names.is_empty() {
                        "No configured profiles.".into()
                    } else {
                        names.join("\n")
                    },
                },
                format,
            );
            Ok(0)
        }
        ConnectionCommand::Test { profile } => {
            not_implemented(format, format!("connection test for profile '{profile}'"))
        }
        ConnectionCommand::Schema { profile, refresh } => not_implemented(
            format,
            format!(
                "schema {} for profile '{profile}'",
                if refresh { "refresh" } else { "inspection" }
            ),
        ),
    }
}

fn ask(
    prompt: Option<String>,
    file: Option<PathBuf>,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let prompt = read_prompt(prompt, file)?;
    if prompt.trim().is_empty() {
        return Err("ask requires a prompt or --file".into());
    }
    not_implemented(
        format,
        format!("AI/database execution for prompt '{prompt}'"),
    )
}

fn query(
    sql: Option<String>,
    file: Option<PathBuf>,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let sql = read_prompt(sql, file)?;
    if sql.trim().is_empty() {
        return Err("query requires --sql or --file".into());
    }
    not_implemented(format, "database query execution".into())
}

fn read_prompt(
    value: Option<String>,
    file: Option<PathBuf>,
) -> Result<String, Box<dyn std::error::Error>> {
    match (value, file) {
        (Some(value), None) => Ok(value),
        (None, Some(path)) => Ok(fs::read_to_string(path)?),
        (Some(_), Some(_)) => Err("provide a prompt or --file, not both".into()),
        (None, None) => Err("a prompt or --file is required".into()),
    }
}

fn not_implemented(
    format: RenderFormat,
    feature: String,
) -> Result<i32, Box<dyn std::error::Error>> {
    emit(TerminalEvent::NotImplemented { feature }, format);
    Ok(0)
}
fn emit(event: TerminalEvent, format: RenderFormat) {
    let rendered = render_event(&event, format);
    print!("{}", rendered.stdout);
    eprint!("{}", rendered.stderr);
}

const CONFIG_TEMPLATE: &str = "default_profile = \"analytics\"\n\n[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\nallow_data_sharing = false\n\n[run]\nread_only = true\nmax_rows = 1000\n";
