use super::session_paths::default_session_dir;
use super::{
    session_commands::SessionAction,
    session_request::PromptResult,
    session_resume::{SessionDefaults, block_on, load_session},
    session_runtime::SessionRuntime,
};
use crate::{
    Cli, GlobalOptions, RenderFormat, RuntimeConfig, SessionState, config,
    slash::{SlashCommand, parse_slash_command},
};
use saya_store::{FsSessionStore, SessionStore, SqliteStateStore};
use std::io::{self, IsTerminal, Write};

/// Runs the interactive session loop.
///
/// When attached to a terminal, each prompt is preceded by a one-line status
/// header (active profile, included databases, provider/model, approval mode,
/// workspace root, and privacy state) and the `saya> ` input marker. Normal
/// terminal scrollback is preserved. Piped input reads lines without the
/// status header, so scripts and CI behave predictably.
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
    let fresh = !cli.options.continue_session && cli.options.resume.is_none();
    if fresh {
        state.provider = runtime.resolved.ai.provider.as_str().into();
        state.model = runtime.resolved.ai.model.clone();
        state.allow_data_sharing = runtime.resolved.ai.allow_data_sharing;
        state.approval_mode = config::runtime::approval_name(&cli.options)?;
        state.included_profiles = cli.options.include_profiles.clone();
    } else {
        // A resumed session keeps its persisted settings, but an explicit
        // `--approval-mode` overrides the persisted mode; without the flag,
        // resume continuity keeps the persisted mode.
        state.approval_mode = resume_approval_mode(&cli.options, &state.approval_mode)?;
    }
    // Always derived, including on a resumed session: the toggle is a display
    // preference that is never persisted, so a resumed session deserializes it
    // as off and would otherwise ignore both the config setting and the flag.
    state.show_thinking = runtime.resolved.ai.show_thinking || cli.options.show_thinking;
    // Reflect the configured default profile so the status bar and @-references
    // match the database the agent actually queries.
    if state.profile.is_none() {
        state.profile = runtime.resolved.profile_name.clone();
    }
    // The session's engine side, once per process: the state directory
    // (`sessions/<id>/`), the single-writer lock, and the tool universe —
    // workspace binding, scratch, fetch, runner — plus the session's one
    // approval policy, built from the session's mode. Held for the process;
    // the lock releases when it drops.
    let mut session = SessionRuntime::acquire(
        &runtime,
        cli.options.workspace.as_deref(),
        fresh,
        state.workspace_root.as_deref(),
        &state.id,
        state
            .approval_mode
            .parse()
            .unwrap_or(saya_agent::ApprovalPolicy::Ask),
        &default_session_dir(),
    )?;
    // The pin the record carries: resolved fresh, or re-bound by an explicit
    // `--workspace`; a resumed session re-opening its recorded pin keeps it
    // untouched (even where the root has vanished, so the record remembers).
    if let Some(root) = session.record_root(fresh) {
        state.workspace_root = Some(root);
    }
    // The launch stated the mode; a fresh session under bypass — or a
    // resume whose `--approval-mode` explicitly overrode the record — is an
    // activation, and the journal records it here, before anything runs. A
    // resume that merely carries the persisted mode re-prints the line but
    // consents to nothing new, so nothing is journalled.
    if super::session_activation::bypass_activated_at_launch(
        fresh,
        cli.options.approval_mode.is_some(),
        &state.approval_mode,
    ) {
        session.journal_bypass_activation(saya_store::BypassSource::Launch);
    }
    let terminal = io::stdin().is_terminal();
    if terminal {
        // Interactive terminals get the full-screen TUI. Chart temp files are
        // removed when the session ends, on the clean path and on error alike.
        let outcome = super::tui::run(
            &runtime,
            &store,
            &state_db,
            format,
            &mut state,
            &mut session,
        );
        crate::chart::cleanup_session_charts();
        let code = outcome?;
        block_on(store.save(state.redacted()))?;
        return Ok(code);
    }
    // Piped / non-TTY input (scripts, CI) uses the headless line executor.
    let outcome = run_plain_loop(
        terminal,
        &mut state,
        &runtime,
        &store,
        &state_db,
        format,
        &mut session,
    );
    crate::chart::cleanup_session_charts();
    outcome?;
    block_on(store.save(state.redacted()))?;
    Ok(0)
}

/// The approval mode a resumed session runs under: an explicit
/// `--approval-mode` overrides the persisted mode; without the flag the
/// persisted mode is kept (resume continuity).
pub(crate) fn resume_approval_mode(
    options: &GlobalOptions,
    persisted: &str,
) -> Result<String, config::runtime::RuntimeError> {
    if options.approval_mode.is_some() {
        config::runtime::approval_name(options)
    } else {
        Ok(persisted.to_owned())
    }
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
    session: &mut SessionRuntime,
) -> Result<(), Box<dyn std::error::Error>> {
    // A startup fact the user must read once, in the loop they will see every
    // turn: a pinned root that vanished, or any other composition notice —
    // and, under bypass, the mode's activation line with its no-euphemism
    // wording and the probe/absence facts.
    if let Some(notice) = session.notice() {
        println!("{notice}");
    }
    if let Some(line) =
        super::session_activation::line_if_bypass(state, runtime, &session.universe())
    {
        println!("{line}");
    }
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
        if handle_line(
            &input, state, runtime, store, state_db, format, terminal, session,
        )? {
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
    session: &mut SessionRuntime,
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
    // A mode change through `/approvals` carries the activation line with it:
    // under bypass the no-euphemism wording, the staged interpreter facts,
    // and the probe's verdict — said where the mode is set, not just implied
    // by the indicator. The mode before the command decides whether this
    // command newly activated bypass: only then is a consent recorded in
    // the journal; a re-statement over an already-bypass session records
    // none, and a failed journal write is said, not silent.
    let before_mode = state.approval_mode.clone();
    let approvals_set = matches!(parsed, Some(SlashCommand::Approvals(Some(_))));
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
            // The turn's decider rides the session's one policy, synced to
            // the current mode (a mid-session `/approval` takes effect next
            // turn, grants carried), so a session grant outlives the turn.
            session.sync_policy(approval);
            match block_on(super::session_request::run(
                runtime,
                line,
                approval,
                session.policy(),
                Some(session.journal()),
                terminal,
                state.prompt_overrides(),
                history,
                format,
                state_db,
                session.universe(),
            )) {
                Ok(PromptResult::Completed(output)) => {
                    state.record_turn(
                        line,
                        output.answer.clone(),
                        output.used_bounded_sql_query,
                        output.tool_metadata.clone(),
                    );
                    // Feed the session accumulator so /usage is honest in
                    // headless mode too (the TUI does this in drain_stream).
                    state.usage.record(&output.usage);
                    // Fold the extraction call's usage into the learning total
                    // before the output moves into the action. `None` (no
                    // extraction or no response) records nothing, so a session
                    // with learning disabled is unaffected.
                    state.usage.record_learning(output.learning_usage);
                    SessionAction::Agent(*output)
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
    if let SessionAction::Doctor = action {
        // Same report the TUI's /doctor shows; runtime lives here.
        super::session_emit::emit_action(
            SessionAction::Message(crate::config::doctor::summary(runtime)),
            format,
            state,
            store,
        )?;
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
    if let SessionAction::Contracts(command) = action {
        // The slash adapter hands the translated `ContractsCommand` to the same
        // `run_contracts` dispatcher the headless `saya contracts` path uses; the
        // captured output goes to the terminal through the shared `emit` seam.
        block_on(crate::commands::run_contracts(
            command, runtime, format, state_db,
        ))?;
        block_on(store.save(state.redacted()))?;
        return Ok(false);
    }
    if let SessionAction::Runs(run_id) = action {
        // `/runs [id]` reaches the same `reads.rs` path the headless
        // `saya run list|show` commands use — the parity contract
        // (tests/run_slash_parity.rs) pins the two adapters byte-identical.
        let command = match run_id {
            Some(run_id) => crate::cli::RunCommand::Show { run_id },
            None => crate::cli::RunCommand::List,
        };
        let approval = state
            .approval_mode
            .parse()
            .map_err(|error: saya_agent::ApprovalPolicyParseError| error.to_string())?;
        block_on(crate::commands::run_management(
            command, runtime, format, approval, state_db,
        ))?;
        block_on(store.save(state.redacted()))?;
        return Ok(false);
    }
    if let SessionAction::RunCancel(run_id) = action {
        // `/run cancel <id>` is the same engine path `saya run cancel` uses —
        // the shared dispatcher, not a second cancellation implementation.
        let approval = state
            .approval_mode
            .parse()
            .map_err(|error: saya_agent::ApprovalPolicyParseError| error.to_string())?;
        block_on(crate::commands::run_management(
            crate::cli::RunCommand::Cancel { run_id },
            runtime,
            format,
            approval,
            state_db,
        ))?;
        block_on(store.save(state.redacted()))?;
        return Ok(false);
    }
    if let SessionAction::Allow(tokens) = action {
        // `/allow <scopes…>` seeds the session's one grant store through the
        // shared behaviour — the same parser, the session surface. A refused
        // scope is an error and seeds nothing; `/allow none` seeds nothing
        // and says so. Each newly seeded token is journalled once by the
        // shared behaviour; a failed journal write changes no grant and is
        // said in the message.
        let action = match super::session_grants::allow(
            &tokens,
            session.policy().grants(),
            &session.journal(),
        ) {
            Ok(message) => SessionAction::Message(message),
            Err(error) => SessionAction::Error(error),
        };
        super::session_emit::emit_action(action, format, state, store)?;
        block_on(store.save(state.redacted()))?;
        return Ok(false);
    }
    if let SessionAction::Grants = action {
        // `/grants` lists the store verbatim: the words are the record, and
        // the mode the store sits under is the engine's own — under bypass
        // it is stated first, so the count never reads "nothing runs".
        super::session_emit::emit_action(
            SessionAction::Message(super::session_grants::listing(
                session.policy().mode(),
                session.policy().grants(),
            )),
            format,
            state,
            store,
        )?;
        block_on(store.save(state.redacted()))?;
        return Ok(false);
    }
    if let SessionAction::Run(tail) = action {
        // `/run --seed-grants <tail…>` seeds the child's `--allow` from this
        // session's grants on request: only tokens the run's parser accepts
        // are forwarded, the rest are named — the parent's own words, before
        // the child's stream begins. The child's parser stays the authority
        // on everything it receives. The nested run's stream passes through
        // on the real stdout/stderr (see `session_run` for the passthrough
        // rule); the child's settle message is the outcome.
        let (seed_requested, tail) = super::session_run::separate_seed_flag(&tail);
        let seed = if seed_requested {
            let seed = super::session_grants::run_seed(&session.policy().grants().tokens());
            super::session_emit::emit_action(
                SessionAction::Message(super::session_grants::seed_message(&seed)),
                format,
                state,
                store,
            )?;
            seed
        } else {
            super::session_grants::RunSeed {
                forwarded: Vec::new(),
                dropped: Vec::new(),
            }
        };
        super::session_run::spawn_run_child(runtime, format, state, &seed.forwarded, tail)?;
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
                // The resumed session's own state must ride the swap: the new
                // state directory claimed and composed before the old lock
                // releases; a refused swap keeps this session. The resumed
                // session's policy is its own — grants are process-lifetime
                // facts about one session, and a resumed session starts empty.
                match session.reacquire(
                    runtime,
                    loaded.workspace_root.as_deref(),
                    &id,
                    loaded
                        .approval_mode
                        .parse()
                        .unwrap_or(saya_agent::ApprovalPolicy::Ask),
                ) {
                    Ok(()) => {
                        *state = loaded;
                        super::session_emit::emit_action(
                            SessionAction::Message(format!("Resumed session {id}")),
                            format,
                            state,
                            store,
                        )?;
                        if let Some(notice) = session.notice() {
                            super::session_emit::emit_action(
                                SessionAction::Message(notice.to_string()),
                                format,
                                state,
                                store,
                            )?;
                        }
                        // A resumed bypass session re-prints its activation
                        // line — the mode is real again, and the user reads
                        // its wording once, not a bare `approval:bypass` on
                        // the status bar.
                        if let Some(line) = super::session_activation::line_if_bypass(
                            state,
                            runtime,
                            &session.universe(),
                        ) {
                            super::session_emit::emit_action(
                                SessionAction::Message(line),
                                format,
                                state,
                                store,
                            )?;
                        }
                    }
                    Err(error) => super::session_emit::emit_action(
                        SessionAction::Error(error),
                        format,
                        state,
                        store,
                    )?,
                }
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
    if approvals_set {
        if let Some(line) =
            super::session_activation::line_if_bypass(state, runtime, &session.universe())
        {
            super::session_emit::emit_action(SessionAction::Message(line), format, state, store)?;
        }
        if super::session_activation::bypass_activated_by_command(
            &before_mode,
            &state.approval_mode,
        ) && let Err(error) = session
            .journal()
            .bypass_activated(saya_store::BypassSource::Command)
        {
            super::session_emit::emit_action(
                SessionAction::Message(super::session_grants::journal_warning(&error)),
                format,
                state,
                store,
            )?;
        }
    }
    block_on(store.save(state.redacted()))?;
    Ok(false)
}
