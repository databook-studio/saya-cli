use crate::{
    cli::ConfigCommand, config_doctor, config_runtime::RuntimeConfig, render::RenderFormat,
};

use super::output::result;

pub(super) fn run(
    command: ConfigCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        ConfigCommand::Init => {
            print!("{CONFIG_TEMPLATE}");
            Ok(0)
        }
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

const CONFIG_TEMPLATE: &str = "default_profile = \"analytics\"\n\n[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\nallow_data_sharing = false\n\n[run]\nread_only = true\nmax_rows = 1000\n";
