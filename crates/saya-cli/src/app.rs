use crate::{cli::Cli, commands, config_runtime, interactive};
use std::path::Path;

pub fn run(cli: Cli) -> i32 {
    match dispatch(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Error: {error}");
            2
        }
    }
}

fn dispatch(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let Some(command) = cli.command.clone() else {
        if cli.options.non_interactive {
            return Err("non-interactive mode requires a subcommand".into());
        }
        return interactive::run(cli);
    };
    let runtime = config_runtime::load(&cli.options, Path::new("."))?;
    let format = config_runtime::format_name(&cli.options, &runtime.resolved);
    commands::run(command, &runtime, format)
}
