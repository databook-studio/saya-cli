use super::*;
use clap::CommandFactory as _;

pub(super) fn dispatch(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let Some(command) = cli.command.clone() else {
        warn_interrupted_setup();
        if cli.options.non_interactive {
            return Err("non-interactive mode requires a subcommand".into());
        }
        crate::local_metrics::set_enabled(cli.options.verbose);
        return interactive::run(cli);
    };
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
    // The MCP server (ADR 0008) also dispatches before the interrupted-setup
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
    // `saya open` stages a local file before any runtime load: it needs no
    // resolved config, generates its own connections file, and launches the
    // session through the same overrides `saya demo` uses.
    if let Command::Open {
        file,
        delimiter,
        no_header,
        reset,
        typed,
        list,
        cleanup,
    } = command
    {
        return open::run(
            &cli,
            open::OpenInvocation {
                file: file.as_deref(),
                delimiter: delimiter.as_deref(),
                no_header,
                reset,
                typed,
                list,
                cleanup: cleanup.as_deref(),
            },
        );
    }
    // `--verbose` seeds the extraction-boundary trace before any turn runs.
    // Until now the flag was declared and read nowhere, so passing it did
    // nothing and said nothing — a small dishonesty in the one surface a user
    // reaches for when memory "didn't record".
    if cli.options.verbose {
        crate::agent::extraction_trace::enable();
        crate::local_metrics::set_enabled(true);
    }
    refuse_continue_on_run(&command, cli.options.continue_session)?;
    refuse_workspace_on_subcommand(Some(&command), cli.options.workspace.as_deref())?;
    refuse_turn_file_on_subcommand(cli.options.turn_file.as_deref())?;
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
