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
    if let Command::Config {
        command: ConfigCommand::Init { project },
    } = &command
    {
        return commands::run_config_init(cli.options.format.into(), *project);
    }
    // `--verbose` seeds the extraction-boundary trace before any turn runs.
    // Until now the flag was declared and read nowhere, so passing it did
    // nothing and said nothing — a small dishonesty in the one surface a user
    // reaches for when memory "didn't record".
    if cli.options.verbose {
        crate::agent::extraction_trace::enable();
    }
    refuse_continue_on_run(&command, cli.options.continue_session)?;
    refuse_workspace_on_subcommand(Some(&command), cli.options.workspace.as_deref())?;
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

/// The `--continue` guard on the run path. The flag continues the
/// interactive REPL session — the only surface that reads it — and a run is
/// not a session: it resumes by explicit id. The flag is not global, so
/// `saya run --continue` is already a clap usage error; this refuses the
/// pre-subcommand spelling rather than letting a run silently ignore a
/// stated intent.
fn refuse_continue_on_run(command: &Command, continue_session: bool) -> Result<(), &'static str> {
    if continue_session && matches!(command, Command::Run { .. }) {
        return Err(
            "`--continue` continues the interactive REPL session, not a run; a run resumes \
             by explicit id: `saya run resume <id>` (`saya run list` prints the ids)",
        );
    }
    Ok(())
}

/// The `--workspace` guard. The flag binds the interactive session's
/// workspace — the only surface that reads it, and only when no subcommand
/// is present — and a subcommand is not a session: `saya ask` composes no
/// session state at all. The flag is not global, so `saya ask --workspace
/// <dir>` is already a clap usage error; this refuses the pre-subcommand
/// spelling rather than letting a subcommand silently ignore a stated
/// intent.
fn refuse_workspace_on_subcommand(
    command: Option<&Command>,
    workspace: Option<&Path>,
) -> Result<(), String> {
    if command.is_some() && workspace.is_some() {
        return Err(
            "`--workspace` binds the interactive session's workspace, not a subcommand: \
             launch the session (`saya --workspace <dir>`) or run the subcommand without it"
                .into(),
        );
    }
    Ok(())
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
    // Runs are headless by construction: they never prompt for a tool call,
    // so an unset approval mode reads `read-only` — read-shaped tools run,
    // anything needing an interactive decision is denied. An explicit
    // --approval-mode (or --non-interactive's "never") wins unchanged.
    if matches!(command, Command::Run { .. }) && options.approval_mode.is_none() {
        options.approval_mode = Some("read-only".to_string());
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    /// `--continue` continues the REPL session only; a run must never
    /// receive it silently. The flag is declared for the bare REPL (not
    /// global, so `saya run --continue` is a clap error), and the
    /// pre-subcommand spelling is refused here with run-shaped guidance.
    #[test]
    fn continue_before_a_run_subcommand_is_refused() {
        let cli = Cli::try_parse_from(["saya", "--continue", "run", "goal", "--allow", "none"])
            .expect("the flag parses before a subcommand");
        assert!(matches!(cli.command, Some(Command::Run { .. })));
        let error = refuse_continue_on_run(
            cli.command.as_ref().expect("the run command parsed"),
            cli.options.continue_session,
        )
        .expect_err("`--continue` before a run must refuse");
        assert!(
            error.contains("saya run resume"),
            "the refusal must name the run-shaped analog: {error}"
        );
    }

    /// `--workspace` reaches only the bare REPL; a subcommand is not a
    /// session, so the pre-subcommand spelling is refused rather than
    /// silently ignored.
    #[test]
    fn workspace_before_a_subcommand_is_refused() {
        let cli = Cli::try_parse_from(["saya", "--workspace", "/tmp/proj", "ask", "question"])
            .expect("the flag parses before a subcommand");
        assert!(cli.command.is_some());
        let error = refuse_workspace_on_subcommand(
            Some(cli.command.as_ref().unwrap()),
            cli.options.workspace.as_deref(),
        )
        .expect_err("`--workspace` before a subcommand must refuse");
        assert!(
            error.contains("interactive session"),
            "the refusal names the surface that reads the flag: {error}"
        );
        // The bare REPL keeps the flag: no subcommand, no guard fires.
        assert!(
            refuse_workspace_on_subcommand(None, Some(Path::new("/tmp/proj"))).is_ok(),
            "no subcommand: the flag binds the session"
        );
    }

    /// The bare REPL keeps the flag: `saya --continue` parses with no
    /// subcommand (the branch dispatch serves before this guard runs, so
    /// only the parse shape is pinned here; the run-shaped refusal is the
    /// test above).
    #[test]
    fn continue_without_a_subcommand_still_reaches_the_repl() {
        let cli = Cli::try_parse_from(["saya", "--continue"]).expect("bare `--continue` parses");
        assert!(cli.command.is_none(), "no subcommand: the REPL path");
        assert!(cli.options.continue_session);
    }
}
