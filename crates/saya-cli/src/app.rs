use crate::{
    cli::{Cli, Command, ConfigCommand, ConnectionCommand},
    commands, config, interactive,
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

use clap::CommandFactory as _;

fn dispatch(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let Some(command) = cli.command.clone() else {
        if cli.options.non_interactive {
            return Err("non-interactive mode requires a subcommand".into());
        }
        return interactive::run(cli);
    };
    if let Command::Completions { shell } = command {
        use clap_complete::generate;
        let mut cmd = Cli::command();
        generate(shell, &mut cmd, "saya", &mut std::io::stdout());
        return Ok(0);
    }
    if matches!(
        &command,
        Command::Config {
            command: ConfigCommand::Init
        }
    ) {
        return commands::run_config_init(cli.options.format.into());
    }
    // `--verbose` seeds the extraction-boundary trace before any turn runs.
    // Until now the flag was declared and read nowhere, so passing it did
    // nothing and said nothing — a small dishonesty in the one surface a user
    // reaches for when memory "didn't record".
    if cli.options.verbose {
        crate::agent::extraction_trace::enable();
    }
    let options = command_options(&cli.options, &command);
    let runtime = config::runtime::load(&options, Path::new("."))?;
    let approval = config::runtime::approval_mode(&options)?;
    let format = config::runtime::format_name(&options, &runtime.resolved);
    let can_prompt = !options.non_interactive && std::io::stdin().is_terminal();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(commands::run(
            command,
            &runtime,
            format,
            approval,
            can_prompt,
            options.include_profiles.clone(),
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
