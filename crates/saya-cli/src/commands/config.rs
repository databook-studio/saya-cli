use crate::cli::{ConfigCommand, DoctorTaskArg};
use crate::config;
use crate::config::doctor_task::{DoctorTask, TaskReadiness, readiness};
use crate::config::runtime::RuntimeConfig;
use crate::render::{RenderFormat, TerminalEvent};

use super::output::{emit, failure_message, result};

pub(super) fn run(
    command: ConfigCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        ConfigCommand::Init { project } => run_init(format, project),
        ConfigCommand::Doctor { task } => run_doctor(runtime, format, task),
        ConfigCommand::Show => {
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

impl From<DoctorTaskArg> for DoctorTask {
    fn from(arg: DoctorTaskArg) -> Self {
        match arg {
            DoctorTaskArg::Query => Self::Query,
            DoctorTaskArg::Ai => Self::Ai,
            DoctorTaskArg::Coding => Self::Coding,
        }
    }
}

/// Without `--task` the flat report keeps its bytes and exit code; with one,
/// readiness is one structured `doctor_task` event and exit 0 (ready) / 3
/// (not ready). `cwd` comes from the process, never `--workspace`, which
/// subcommands refuse.
fn run_doctor(
    runtime: &RuntimeConfig,
    format: RenderFormat,
    task: Option<DoctorTaskArg>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let Some(arg) = task else {
        let diagnosis = crate::config::doctor::report(runtime);
        result(diagnosis.lines.join("\n"), format)?;
        return Ok(diagnosis.exit_code());
    };
    let task = DoctorTask::from(arg);
    let cwd = std::env::current_dir()?;
    let readiness: TaskReadiness = readiness(runtime, task, &cwd);
    emit(
        TerminalEvent::DoctorTask {
            task: task.name().to_string(),
            ready: readiness.ready,
            checks: readiness.checks,
        },
        format,
    );
    Ok(if readiness.ready { 0 } else { 3 })
}

pub(super) fn run_init(
    format: RenderFormat,
    project: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir();
    let message = match cwd {
        Ok(cwd) if project => config::init::create_project_files(&cwd),
        Ok(cwd) => {
            // Default: the trusted user layer, so a following command does not
            // warn. If a project config already exists,
            // append a one-line hint (not a migration) — folded into the result
            // message so the structured --format envelopes stay on stdout and
            // stderr stays empty.
            let user_dir = config::sources::user_config_dir();
            match config::init::create_user_files(&user_dir) {
                Ok(message) => Ok(notice_existing_project_config(message, &cwd)),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    match message {
        Ok(message) => result(message, format),
        Err(error) => failure_message(2, config::init::error_message(&error), format),
    }
}

/// If the cwd already has a `.saya/config.toml`, say so once. The default
/// `config init` writes the trusted user layer; the existing project config
/// stays untrusted, and a user who meant to refresh it has `--project` and
/// `saya config doctor` to reach for.
fn notice_existing_project_config(mut message: String, cwd: &std::path::Path) -> String {
    if cwd.join(".saya/config.toml").exists() {
        message.push_str(
            "\nNote: this project already has a .saya/config.toml. It stays \
             untrusted; run `saya config doctor` to see how to apply it, or \
             `saya config init --project` to refresh the project templates.",
        );
    }
    message
}
