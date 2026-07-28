use crate::{
    cli::{Cli, Command, ConnectionCommand},
    commands, config_runtime, interactive,
};
use std::{io::IsTerminal, path::Path};

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
    let options = command_options(&cli.options, &command);
    let runtime = config_runtime::load(&options, Path::new("."))?;
    let approval = config_runtime::approval_mode(&options)?;
    let format = config_runtime::format_name(&options, &runtime.resolved);
    let can_prompt = !options.non_interactive && std::io::stdin().is_terminal();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(commands::run(
            command, &runtime, format, approval, can_prompt,
        ))
}

fn command_options(
    options: &crate::cli::GlobalOptions,
    command: &Command,
) -> crate::cli::GlobalOptions {
    let mut options = options.clone();
    if options.profile.is_none() {
        let profile = match command {
            Command::Connection {
                command:
                    ConnectionCommand::Test { profile_name }
                    | ConnectionCommand::Schema { profile_name, .. },
            } => Some(profile_name.clone()),
            _ => None,
        };
        options.profile = profile;
    }
    options
}
