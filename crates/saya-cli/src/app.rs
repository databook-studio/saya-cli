use crate::{
    cli::{Cli, Command, ConfigCommand, ConnectionCommand, McpCommand},
    commands, config, demo, interactive,
};
use std::{io::IsTerminal, path::Path};

mod dispatch;
mod print;
use dispatch::dispatch;

pub fn run(cli: Cli) -> i32 {
    match dispatch(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Error: {error}");
            2
        }
    }
}

/// Startup warning: if an earlier `saya setup` was interrupted mid-commit,
/// every command except `setup` (which offers the recovery interactively) and
/// `completions` (which must keep its stdout contract clean) says so once, to
/// stderr. Never auto-restores; a marker read error is a warning, never a crash.
fn warn_interrupted_setup() {
    let dir = config::sources::user_config_dir();
    match crate::setup::pending(&dir) {
        Ok(Some(_)) => eprintln!(
            "An interrupted `saya setup` left a recovery marker in {}. \
             Run `saya setup` to restore or finish it.",
            dir.display()
        ),
        Ok(None) => {}
        Err(error) => {
            eprintln!("warning: could not check for an interrupted `saya setup`: {error}")
        }
    }
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

/// The session-launch guard. `--allow` states the interactive session's
/// launch grants — the only surface that reads them. A subcommand is not a
/// session: `saya ask` composes no session state at all and `saya run` is
/// headless by construction, so both refuse the flags rather than silently
/// ignoring a stated intent.
pub(crate) fn refuse_session_launch_flags_on_subcommand(
    command: &Command,
    options: &crate::cli::GlobalOptions,
) -> Result<(), String> {
    if !options.deny.is_empty() {
        return match command {
            Command::Ask { .. } => {
                Err(crate::interactive::session::permissions::deny::ask_surface_refusal())
            }
            Command::Run { .. } => Err(crate::commands::run::scopes::refuse_deny_on_run()),
            _ => Err(
                "`--deny` states the interactive session's deny list, not a subcommand: \
                 launch the session (`saya --deny <program>`) or run the subcommand without it"
                    .into(),
            ),
        };
    }
    if !options.allow.is_empty() {
        return Err(match command {
            Command::Ask { .. } => {
                "`--allow` states the interactive session's launch grants, not a one-shot ask: \
                 launch the session (`saya --allow <scopes>`) or run the ask without it"
                    .to_owned()
            }
            Command::Run { .. } => {
                "`--allow` on `saya run` is the run's own flag, not the session's: \
                 `saya run --allow <scopes>` states the run's scopes"
                    .to_owned()
            }
            _ => "`--allow` states the interactive session's launch grants, not a subcommand: \
                 launch the session (`saya --allow <scopes>`) or run the subcommand without it"
                .to_owned(),
        });
    }
    Ok(())
}

/// The `--tools` guard. The flag narrows the model tools an *agent surface*
/// advertises — the interactive session (including the `saya demo` launch),
/// `saya ask`, `saya run`, and `saya start`. Every other subcommand has no
/// tool surface to narrow, so it refuses the flag rather than parsing it and
/// silently dropping a stated intent.
pub(crate) fn refuse_tools_on_subcommand(
    command: Option<&Command>,
    options: &crate::cli::GlobalOptions,
) -> Result<(), &'static str> {
    if options.tools.is_empty() || command.is_none_or(honours_tools) {
        return Ok(());
    }
    Err(crate::cli::TOOLS_REFUSAL)
}

/// Whether this subcommand composes a tool surface `--tools` can narrow.
fn honours_tools(command: &Command) -> bool {
    match command {
        Command::Ask { .. } | Command::Run { .. } | Command::Demo { .. } => true,
        // `saya start` is the only start spelling: one list governs it.
        Command::Start(_) => true,
        _ => false,
    }
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

/// The `--turn-file` guard. The flag reads one turn for the interactive
/// session — the only surface that reads it, and only when no subcommand is
/// present. A subcommand is not a session, so the pre-subcommand spelling is
/// refused rather than silently ignoring a stated intent.
fn refuse_turn_file_on_subcommand(turn_file: Option<&Path>) -> Result<(), String> {
    if turn_file.is_some() {
        return Err(
            "`--turn-file` reads one turn for the interactive session, not a subcommand: \
             launch the session (`saya --turn-file <path>`) or run the subcommand without it"
                .into(),
        );
    }
    Ok(())
}

/// The `--permission-mode` guard. The flag starts the interactive session in
/// one of the session's four modes — the only surface that reads it, and only
/// when no subcommand is present. A subcommand is not a session, so the
/// pre-subcommand spelling is refused rather than silently ignoring a stated
/// intent.
fn refuse_permission_mode_on_subcommand(permission_mode: Option<&str>) -> Result<(), String> {
    if permission_mode.is_some() {
        return Err(
            "`--permission-mode` starts the interactive session in a permission mode, not a \
             subcommand: launch the session (`saya --permission-mode <mode>`) or run the \
             subcommand without it"
                .into(),
        );
    }
    Ok(())
}

/// `--permission-mode` and `--approval-mode` are two spellings of the same
/// consent: one names the session's mode, the other the posture it decomposes
/// to. Stating both is refused rather than resolved by precedence — the pair
/// would depend on which check ran first.
pub(crate) fn refuse_conflicting_consent(
    permission_mode: Option<&str>,
    approval_mode: Option<&str>,
) -> Result<(), String> {
    if let (Some(mode), Some(approval)) = (permission_mode, approval_mode) {
        return Err(format!(
            "`--permission-mode {mode}` and `--approval-mode {approval}` both state the session's \
             consent: state one of them, or switch inside the session with `/mode`"
        ));
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

    /// `saya --host-commands` is a clap usage error: the flag
    /// does not exist, and clap's honest answer to a stale invocation is
    /// exit 2.
    #[test]
    fn host_commands_flag_is_now_a_usage_error() {
        for argv in [
            vec!["saya", "--host-commands"],
            vec!["saya", "--host-commands", "ask", "count orders"],
        ] {
            let error =
                Cli::try_parse_from(argv).expect_err("--host-commands must not parse after G2");
            assert!(
                error.kind() == clap::error::ErrorKind::UnknownArgument,
                "a stale flag is a usage error: {error}"
            );
            let rendered = error.to_string();
            assert!(
                rendered.contains("host-commands"),
                "the usage error names the stale spelling: {rendered}"
            );
        }
    }

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

    /// `--permission-mode` starts the session in a mode; a subcommand is not
    /// a session, so the pre-subcommand spelling is refused, and the
    /// post-subcommand one never parses (the flag is not global).
    #[test]
    fn permission_mode_before_a_subcommand_is_refused() {
        let cli = Cli::try_parse_from(["saya", "--permission-mode", "plan", "ask", "question"])
            .expect("the flag parses before a subcommand");
        assert!(cli.command.is_some());
        let error = refuse_permission_mode_on_subcommand(cli.options.permission_mode.as_deref())
            .expect_err("`--permission-mode` before a subcommand must refuse");
        assert!(
            error.contains("interactive session"),
            "the refusal names the surface that reads the flag: {error}"
        );
        assert!(
            refuse_permission_mode_on_subcommand(None).is_ok(),
            "no flag: no guard fires"
        );
        let error = Cli::try_parse_from(["saya", "ask", "--permission-mode", "plan"])
            .expect_err("the flag is not global: a subcommand cannot carry it");
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    /// The two consent spellings are refused together rather than resolved
    /// by precedence: the pair a session would run under must not depend on
    /// which check ran first.
    #[test]
    fn permission_mode_with_approval_mode_is_refused() {
        let error = refuse_conflicting_consent(Some("plan"), Some("ask"))
            .expect_err("two spellings of one consent must refuse");
        assert!(
            error.contains("--permission-mode plan") && error.contains("--approval-mode ask"),
            "the refusal names both flags: {error}"
        );
        assert!(refuse_conflicting_consent(Some("plan"), None).is_ok());
        assert!(refuse_conflicting_consent(None, Some("bypass")).is_ok());
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
