use crate::{
    cli::ConfigCommand, config_doctor, config_init, config_runtime::RuntimeConfig,
    render::RenderFormat,
};

use super::output::{failure_message, result};

pub(super) fn run(
    command: ConfigCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        ConfigCommand::Init => run_init(format),
        ConfigCommand::Doctor => result(config_doctor::summary(runtime), format),
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

pub(super) fn run_init(format: RenderFormat) -> Result<i32, Box<dyn std::error::Error>> {
    match std::env::current_dir().and_then(|cwd| config_init::create_project_files(&cwd)) {
        Ok(message) => result(message, format),
        Err(error) => failure_message(2, config_init::error_message(&error), format),
    }
}
