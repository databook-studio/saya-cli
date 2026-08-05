use super::session_paths::default_session_dir;
use super::{
    session_commands::SessionAction,
    session_request::PromptResult,
    session_resume::{SessionDefaults, block_on, load_session},
};
use crate::{Cli, RenderFormat, RuntimeConfig, SessionState, config, slash::parse_slash_command};
use saya_store::{FsSessionStore, SessionStore, SqliteStateStore};
use std::io::{self, IsTerminal, Write};

/// Runs the interactive session loop.
///
/// When attached to a terminal, each prompt is preceded by a one-line status
/// header (active profile, included databases, provider/model, approval mode,
/// and privacy state) and the `saya> ` input marker. Normal terminal scrollback
/// is preserved. Piped input reads lines without the status header, so scripts
/// and CI behave predictably.
pub fn run(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let runtime = config::runtime::load(&cli.options, std::path::Path::new("."))?;
    let format = config::runtime::format_name(&cli.options, &runtime.resolved);
    let store = FsSessionStore::new(default_session_dir());
    let state_db = SqliteStateStore::new(crate::state_path::state_db_path());
    let defaults = SessionDefaults {
        provider: runtime.resolved.ai.provider.as_str().into(),
        model: runtime.resolved.ai.model.clone(),
        allow_data_sharing: runtime.resolved.ai.allow_data_sharing,
        approval_mode: config::runtime::approval_name(&cli.options)?,
    };
    let mut state = load_session(&store, &cli, &defaults)?;
    if !cli.options.continue_session && cli.options.resume.is_none() {
        state.provider = runtime.resolved.ai.provider.as_str().into();
        state.model = runtime.resolved.ai.model.clone();
        state.allow_data_sharing = runtime.resolved.ai.allow_data_sharing;
        state.approval_mode = config::runtime::approval_name(&cli.options)?;
        state.included_profiles = cli.options.include_profiles.clone();
    }
    // Reflect the configured default profile so the status bar and @-references
    // match the database the agent actually queries.
    if state.profile.is_none() {
        state.profile = runtime.resolved.profile_name.clone();
    }
    let terminal = io::stdin().is_terminal();
    if terminal {
        // Interactive terminals get the full-screen TUI.
        let code = super::tui::run(&runtime, &store, &state_db, format, &mut state)?;
        block_on(store.save(state.redacted()))?;
        return Ok(code);
    }
    // Piped / non-TTY input (scripts, CI) uses the headless line executor.
    run_plain_loop(terminal, &mut state, &runtime, &store, &state_db, format)?;
    block_on(store.save(state.redacted()))?;
    Ok(0)
}

/// Reads lines from stdin without the rich editor, printing the status header
/// and `saya> ` marker when attached to a terminal. Used for piped input and as
/// a graceful fallback when the rich editor cannot initialize.
fn run_plain_loop(
    terminal: bool,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    state_db: &SqliteStateStore,
    format: RenderFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    loop {
        if terminal {
            println!("{}", super::session_prompt::status_line(state));
            print!("saya> ");
            io::stdout().flush()?;
        }
        input.clear();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }
        if handle_line(&input, state, runtime, store, state_db, format, terminal)? {
            break;
        }
    }
    Ok(())
}

/// Processes one input line: dispatches a slash command or an agent prompt,
/// renders the resulting action, and persists the redacted session. Returns
/// `Ok(true)` when the session should exit.
#[allow(clippy::too_many_arguments)]
fn handle_line(
    line: &str,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    state_db: &SqliteStateStore,
    format: RenderFormat,
    terminal: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let line = line.trim_end();
    if line.trim().is_empty() {
        return Ok(false);
    }
    // A malformed or unknown slash command must not tear down the whole session:
    // surface the parse error (which may carry a "did you mean" hint) and keep looping.
    let parsed = match parse_slash_command(line) {
        Ok(parsed) => parsed,
        Err(error) => {
            super::session_emit::emit_action(
                SessionAction::Error(error.to_string()),
                format,
                state,
                store,
            )?;
            block_on(store.save(state.redacted()))?;
            return Ok(false);
        }
    };
    let action = match parsed {
        Some(command) => state.apply(
            command,
            &runtime
                .connections
                .profiles
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
        ),
        None => {
            let history = state.provider_history();
            let approval = state
                .approval_mode
                .parse()
                .map_err(|error: saya_agent::ApprovalPolicyParseError| error.to_string())?;
            match block_on(super::session_request::run(
                runtime,
                line,
                approval,
                terminal,
                state.prompt_overrides(),
                history,
                format,
                state_db,
            )) {
                Ok(PromptResult::Completed(output)) => {
                    state.record_turn(
                        line,
                        output.answer.clone(),
                        output.used_bounded_sql_query,
                        output.tool_metadata.clone(),
                    );
                    SessionAction::Agent(output)
                }
                Ok(PromptResult::Cancelled) => SessionAction::Cancelled,
                Err(error) => SessionAction::Error(error.to_string()),
            }
        }
    };
    if matches!(action, SessionAction::Exit) {
        return Ok(true);
    }
    if let SessionAction::Schema(refresh) = action {
        block_on(super::session_schema::run(
            runtime,
            state.profile.as_deref(),
            refresh,
            terminal,
            format,
            state_db,
        ))?;
        block_on(store.save(state.redacted()))?;
        return Ok(false);
    }
    if let SessionAction::Sql(sql) = action {
        block_on(super::session_sql::run(
            runtime,
            state.profile.as_deref(),
            &sql,
            terminal,
            format,
        ))?;
        block_on(store.save(state.redacted()))?;
        return Ok(false);
    }
    if let SessionAction::Resume(id) = action {
        let defaults = super::session_resume::SessionDefaults {
            provider: state.provider.clone(),
            model: state.model.clone(),
            allow_data_sharing: state.allow_data_sharing,
            approval_mode: state.approval_mode.clone(),
        };
        match super::session_resume::resume_session(store, &id, &defaults) {
            Ok(Some(loaded)) => {
                *state = loaded;
                super::session_emit::emit_action(
                    SessionAction::Message(format!("Resumed session {id}")),
                    format,
                    state,
                    store,
                )?;
            }
            Ok(None) => super::session_emit::emit_action(
                SessionAction::Error(format!("Session not found: {id}")),
                format,
                state,
                store,
            )?,
            Err(error) => super::session_emit::emit_action(
                SessionAction::Error(error.to_string()),
                format,
                state,
                store,
            )?,
        }
        block_on(store.save(state.redacted()))?;
        return Ok(false);
    }
    super::session_emit::emit_action(action, format, state, store)?;
    block_on(store.save(state.redacted()))?;
    Ok(false)
}
