use super::*;
use clap::CommandFactory as _;

pub(super) fn dispatch(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    // The retired `saya coding` spelling answers before any guard or config
    // load: it names where the verb moved, and it must not be reinterpreted
    // as a surface that reads a global flag (or as a runtime/config error).
    if let Some(Command::Coding { command }) = cli.command.as_ref() {
        return commands::failure_message(
            2,
            crate::cli::retired::moved_message(command.as_ref()),
            cli.options.format.into(),
        );
    }
    // Before every branch below, including the ones that dispatch early
    // (`completions`, `setup`, `runtime`, `mcp serve`, `config init`): the
    // flag is a global arg, so clap accepts it on any subcommand, and a
    // subcommand with no tool surface refuses it rather than dropping it.
    refuse_tools_on_subcommand(cli.command.as_ref(), &cli.options)?;
    // `-p` is a whole launch: a subcommand beside it is a second intent,
    // refused before either surface runs.
    if let Err(message) =
        crate::cli::print::refuse_subcommand(cli.command.as_ref(), cli.print.task.as_deref())
    {
        return commands::failure_message(2, message, cli.options.format.into());
    }
    let Some(command) = cli.command.clone() else {
        warn_interrupted_setup();
        if cli.print.task.is_some() {
            return super::print::dispatch(cli);
        }
        if cli.options.non_interactive {
            return Err("non-interactive mode requires a subcommand".into());
        }
        crate::local_metrics::set_enabled(cli.options.verbose);
        return interactive::run(cli);
    };
    // Runtime install/check commands are explicit local operator actions.
    // Dispatch them before setup recovery, config resolution, provider setup,
    // or state-store creation; their owner controls any local effects.
    if let Command::Runtime {
        command: runtime_command,
    } = &command
    {
        refuse_workspace_on_subcommand(Some(&command), cli.options.workspace.as_deref())?;
        refuse_turn_file_on_subcommand(cli.options.turn_file.as_deref())?;
        refuse_permission_mode_on_subcommand(cli.options.permission_mode.as_deref())?;
        refuse_session_launch_flags_on_subcommand(&command, &cli.options)?;
        return match runtime_command {
            crate::cli::RuntimeCommand::InstallNode { yes: false } => commands::failure_message(
                2,
                "Node.js and npm installation requires explicit --yes; no files were changed."
                    .to_owned(),
                cli.options.format.into(),
            ),
            crate::cli::RuntimeCommand::InstallNode { yes: true } => {
                commands::runtime::install_node(cli.options.format.into())
            }
            crate::cli::RuntimeCommand::InstallDeps { yes: false, .. } => {
                commands::failure_message(
                    2,
                    "npm dependency acquisition requires explicit --yes; no files were changed."
                        .to_owned(),
                    cli.options.format.into(),
                )
            }
            crate::cli::RuntimeCommand::InstallDeps { project, yes: true } => {
                commands::runtime::install_deps(project.clone(), cli.options.format.into())
            }
            crate::cli::RuntimeCommand::CheckNode { yes: false, .. } => commands::failure_message(
                2,
                "running a project check requires explicit --yes; no files were changed."
                    .to_owned(),
                cli.options.format.into(),
            ),
            crate::cli::RuntimeCommand::CheckNode {
                project,
                action,
                yes: true,
            } => commands::runtime::check_node(project.clone(), *action, cli.options.format.into()),
        };
    }
    if let Command::Completions { shell } = command {
        use clap_complete::generate;
        let mut cmd = Cli::command();
        generate(shell, &mut cmd, "saya", &mut std::io::stdout());
        return Ok(0);
    }
    // Setup runs the guided flow before any runtime load: it writes the
    // config it then loads, and refuses to prompt when there is no terminal.
    if let Command::Setup = command {
        return crate::setup::flow::run(&cli);
    }
    // The MCP server also dispatches before the interrupted-setup
    // warning: its stderr belongs to the client's log stream and its stdout
    // must stay protocol-only, so `mcp serve` never emits the startup
    // warnings the interactive surfaces do. The session-launch guards still
    // apply — a stated intent is never silently ignored — but the setup
    // recovery offer is a one-line stderr note, not a surface contract.
    if let Command::Mcp {
        command: McpCommand::Serve { profile },
    } = &command
    {
        refuse_workspace_on_subcommand(Some(&command), cli.options.workspace.as_deref())?;
        refuse_turn_file_on_subcommand(cli.options.turn_file.as_deref())?;
        refuse_permission_mode_on_subcommand(cli.options.permission_mode.as_deref())?;
        refuse_session_launch_flags_on_subcommand(&command, &cli.options)?;
        return crate::mcp::serve(&cli.options, profile.clone());
    }
    warn_interrupted_setup();
    if let Command::Config {
        command: ConfigCommand::Init { project },
    } = &command
    {
        return commands::run_config_init(cli.options.format.into(), *project);
    }
    if let Command::Demo { reset } = command {
        return demo::run(&cli, reset);
    }
    // `--verbose` seeds the extraction-boundary trace before any turn runs.
    // The flag seeds the extraction-boundary trace; without this, passing it would
    // do nothing and say nothing — a small dishonesty in the one surface a user
    // reaches for when memory "didn't record".
    if cli.options.verbose {
        crate::agent::extraction_trace::enable();
        crate::local_metrics::set_enabled(true);
    }
    refuse_continue_on_run(&command, cli.options.continue_session)?;
    refuse_workspace_on_subcommand(Some(&command), cli.options.workspace.as_deref())?;
    refuse_turn_file_on_subcommand(cli.options.turn_file.as_deref())?;
    refuse_permission_mode_on_subcommand(cli.options.permission_mode.as_deref())?;
    refuse_session_launch_flags_on_subcommand(&command, &cli.options)?;
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
