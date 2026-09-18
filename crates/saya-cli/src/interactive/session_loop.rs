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
    // The startup trust decision (G3, Decision 2 §1): one decision — prompt
    // when a fresh session bound no root on a terminal — with two
    // renderings. The full-screen TUI renders it as a modal inside the
    // interface after the splash paints (a raw stdin read before it would
    // pre-empt the alternate screen and leave the terminal broken); the
    // plain REPL renders it as the line prompt, asked here before the loop.
    // Terminal-attached TUI sessions defer the question (no stdin read
    // here); every other terminal session asks it here, on stderr.
    let terminal_probe = io::stdin().is_terminal();
    let tui_surface = terminal_probe && cli.options.turn_file.is_none();
    let trusted_dir: Option<std::path::PathBuf> = if tui_surface {
        None
    } else {
        startup_trust_answer(&cli, fresh, state.workspace_root.as_deref(), terminal_probe)
    };
    // A TUI session that still needs the trust answer opens the modal once
    // the interface paints — the same one decision, rendered inside the
    // surface instead of in front of it.
    let tui_trust_pending = tui_surface
        && super::session_trust::should_prompt(&super::session_trust::TrustPromptContext {
            is_terminal: true,
            fresh,
            has_explicit: cli.options.workspace.is_some(),
            has_pin: state.workspace_root.is_some(),
            root_bound: state.workspace_root.is_some()
                || super::session_workspace::resolve_root(
                    None,
                    &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                )
                .ok()
                .flatten()
                .is_some(),
            turn_file: cli.options.turn_file.is_some(),
        });
    let mut trusted_echo: Option<String> = None;
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
    // workspace binding, scratch, fetch, runner, host lane — plus the
    // session's one approval policy, built from the session's mode. Held for
    // the process; the lock releases when it drops.
    //
    // The host lane composes here, once per session: the launch statement
    // (the `--allow command:<x>` seeds — which seed the grant, never imply
    // composition — the `--deny` refusals, the user-layer config) is read,
    // the universe composes with it, and the seeds land in the grant store
    // before anything runs. A seed the composition cannot carry is a launch
    // usage error — never a silently dropped token. A resumed session
    // restarts unstated: grants die with the process, and the launch
    // statement belonged to the previous process.
    let launch = super::session_host::HostLaunch::from_options(&cli.options, &runtime);
    // The launch contradiction: `--allow command:x` together with `--deny
    // x` grants what it refuses — an exit-2 usage error, before anything
    // exists. Every `--deny` entry is a bare name first: a path-shaped,
    // traversal, prefix, or glob entry is a typed launch error.
    for entry in &cli.options.deny {
        if let Err(error) = super::session_deny::validate_deny_entry(entry) {
            return Err(format!("invalid --deny entry: {error}").into());
        }
    }
    for seed in &cli.options.allow {
        if let Some(name) = seed.strip_prefix("command:")
            && cli.options.deny.iter().any(|denied| denied == name)
        {
            return Err(super::session_deny::launch_contradiction(name).into());
        }
    }
    let mut session = SessionRuntime::acquire_inner(super::session_runtime::Acquire {
        runtime: &runtime,
        explicit: cli.options.workspace.as_deref(),
        fresh,
        pinned_root: state.workspace_root.as_deref(),
        id: &state.id,
        mode: state
            .approval_mode
            .parse()
            .unwrap_or(saya_agent::ApprovalPolicy::Ask),
        sessions_root: &default_session_dir(),
        trusted: trusted_dir.as_deref(),
    })?;
    // The trust answer's echo: the moment of choice carries the tree — the
    // half of the pair the bypass activation line's lane fact does not
    // carry. Said once the root actually bound (a `w <dir>` that refused
    // would have errored above, never echoed).
    if session.root().is_some() && trusted_dir.is_some() {
        trusted_echo = session.root().map(super::session_trust::trusted_root_line);
    }
    // Recompose with the launch statement on a fresh start: `acquire`
    // composed without the launch's deny refusals. The deny list rides the
    // same recomposition — refusal-only, composes nothing — so a deny-only
    // launch still composes its refusals. The lane itself composes wherever
    // a root binds regardless, through the same composer — one composer
    // behind both paths — so every fresh start recomposes.
    if fresh {
        let recomposed = super::session_universe::SessionUniverse::compose_with_launch(
            &runtime,
            cli.options.workspace.as_deref().or(trusted_dir.as_deref()),
            state.workspace_root.as_deref(),
            true,
            &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            &session.state_dir(),
            Some(&launch),
        )?;
        session.replace_universe(recomposed);
    }
    // The deny list journals once at session start when non-empty — no noise
    // when empty — through the existing seam, never a second rule set. The
    // status header's facts ride the session state: the lane bit and the
    // deny list, so every prompt carries the `host:` segment.
    if fresh {
        state.host_composed = session.universe().host_composed();
        state.denied_programs = session.universe().deny_programs();
        let denied = session.universe().deny_programs();
        if !denied.is_empty()
            && let Err(error) = session.journal().deny_list(&denied)
        {
            eprintln!("{}", super::session_grants::journal_warning(&error));
        }
    }
    if fresh && !cli.options.allow.is_empty() {
        // The launch helper seeds the store through the same grammar; the
        // shared behaviour below journals each token. A seed the composition
        // cannot carry is a launch usage error — never silently dropped.
        let launch_seeded = launch
            .seed_grants(session.policy().grants())
            .map_err(|error| format!("invalid --allow seed: {error}"))?;
        let seeded = super::session_grants::seed_launch_allow(
            &launch_seeded,
            &session.universe().approval_facts(&runtime),
            session.policy().grants(),
        )
        .map_err(|error| format!("invalid --allow seed: {error}"))?;
        for token in seeded {
            if let Err(error) = session
                .journal()
                .granted(&token, saya_store::GrantSource::Seed)
            {
                eprintln!("{}", super::session_grants::journal_warning(&error));
                break;
            }
        }
    };
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
    let terminal = terminal_probe;
    let trusted_echo_for_tui = trusted_echo.clone();
    if terminal {
        // Interactive terminals get the full-screen TUI. Chart temp files are
        // removed when the session ends, on the clean path and on error alike.
        // The startup trust question rides the TUI's modal when it is still
        // open — asked inside the interface after the splash paints, never
        // as a raw stdin read in front of it. The modal binds through the
        // live runtime when answered; the pin and the header facts below
        // refresh from the rebound session when the TUI returns them.
        let outcome = super::tui::run(super::tui::TuiSession {
            runtime: &runtime,
            store: &store,
            state_db: &state_db,
            format,
            state: &mut state,
            session: &mut session,
            trusted_echo: trusted_echo_for_tui.as_deref(),
            trust_pending: tui_trust_pending,
            launch: &launch,
        });
        if let Ok(super::tui::TrustOutcome::Answered(dir)) = &outcome {
            let _ = dir;
        }
        crate::chart::cleanup_session_charts();
        let code = outcome.map(|outcome| outcome.exit_code())?;
        if let Some(root) = session.record_root(fresh) {
            state.workspace_root = Some(root);
        }
        state.host_composed = session.universe().host_composed();
        state.denied_programs = session.universe().deny_programs();
        block_on(store.save(state.redacted()))?;
        return Ok(code);
    }
    // Piped / non-TTY input (scripts, CI) uses the headless line executor.
    // `--turn-file` runs one verbatim turn instead: the file bytes reach the
    // same per-turn entry (`handle_line`) with no line splitting, no
    // trimming, and no blank-line skipping, then the process exits.
    if let Some(path) = &cli.options.turn_file {
        let turn = match read_turn_file(path) {
            Ok(turn) => turn,
            Err(error) => {
                let rendered = crate::render_event(
                    &crate::TerminalEvent::Error {
                        message: error.clone(),
                    },
                    format,
                );
                print!("{}", rendered.stdout);
                eprint!("{}", rendered.stderr);
                return Err(error.into());
            }
        };
        let mut ctx = TurnContext {
            state: &mut state,
            runtime: &runtime,
            store: &store,
            state_db: &state_db,
            format,
            terminal: false,
            session: &mut session,
            trusted_echo: None,
        };
        let outcome = run_single_turn(turn, &mut ctx);
        crate::chart::cleanup_session_charts();
        block_on(store.save(state.redacted()))?;
        return match outcome? {
            TurnOutcome::Completed => Ok(0),
            TurnOutcome::Errored => Ok(5),
            TurnOutcome::Exit => Ok(0),
        };
    }
    let mut ctx = TurnContext {
        state: &mut state,
        runtime: &runtime,
        store: &store,
        state_db: &state_db,
        format,
        terminal,
        session: &mut session,
        trusted_echo,
    };
    let outcome = run_plain_loop(&mut ctx);
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

/// The startup trust decision (G3): whether this launch asks the trust
/// prompt, and the answered directory when it does. Fresh sessions only —
/// a resume re-opens its recorded pin untouched — with no explicit
/// `--workspace` and no root already bound (an explicit statement or a
/// worktree top answers the question before it is asked). Terminal only:
/// piped stdin, `--turn-file`, and `--format json` have nobody to prompt,
/// so nothing binds there. A refused `w <dir>` is a launch usage error,
/// never a silent unbound session.
fn startup_trust_answer(
    cli: &Cli,
    fresh: bool,
    pinned_root: Option<&str>,
    is_terminal: bool,
) -> Option<std::path::PathBuf> {
    if !fresh || cli.options.workspace.is_some() || pinned_root.is_some() {
        return None;
    }
    if cli.options.turn_file.is_some() {
        return None;
    }
    // JSON output owns stdout for machines: the prompt writes to stderr, but
    // a headless consumer still has nobody to answer — never ask.
    if !is_terminal {
        return None;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    // A worktree top binds without asking: the prompt exists for the unbound
    // shape only, never as inference with a confirmation step.
    let already_binds = super::session_workspace::resolve_root(None, &cwd)
        .ok()
        .flatten()
        .is_some();
    if already_binds {
        return None;
    }
    let ctx = super::session_trust::TrustPromptContext {
        is_terminal,
        fresh,
        has_explicit: false,
        has_pin: false,
        root_bound: false,
        turn_file: false,
    };
    if !super::session_trust::should_prompt(&ctx) {
        return None;
    }
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stderr = io::stderr();
    let answer = super::session_trust::ask_trust(&mut input, &mut stderr)
        .unwrap_or(super::session_trust::TrustAnswer::ContinueUnbound);
    match answer {
        super::session_trust::TrustAnswer::TrustCwd => {
            Some(super::session_trust::resolve_trusted_dir(&cwd).unwrap_or(cwd))
        }
        super::session_trust::TrustAnswer::Workspace(dir) => Some(dir),
        super::session_trust::TrustAnswer::ContinueUnbound => None,
    }
}

/// Reads one turn verbatim from `path`: the bytes reach the turn unaltered —
/// blank lines, trailing whitespace, code fences. Only a trailing `\n` or
/// `\r\n` (the file's own line ending) is stripped, since the per-turn entry
/// below treats a trailing newline as the end of input, not content.
fn read_turn_file(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read --turn-file {}: {error}", path.display()))?;
    let mut text = String::from_utf8(bytes)
        .map_err(|error| format!("--turn-file {} is not valid UTF-8: {error}", path.display()))?;
    if text.ends_with("\r\n") {
        text.truncate(text.len() - 2);
    } else if text.ends_with('\n') {
        text.truncate(text.len() - 1);
    }
    if text.trim().is_empty() {
        return Err(format!("--turn-file {} is empty", path.display()));
    }
    Ok(text)
}

/// The outcome of the single `--turn-file` turn: completed (exit 0),
/// errored (exit 5, the streamed `TerminalEvent::Error` names it), or an
/// explicit `/exit` (still exit 0 — the turn ran and asked to leave).
enum TurnOutcome {
    Completed,
    Errored,
    Exit,
}

/// Everything one headless turn runs with. A bundle rather than seven
/// positional parameters, so the call sites read by name. `state` is the
/// session being turned; `session` is its engine side (universe, policy,
/// journal). `terminal` decides whether the loop prints the status header
/// and whether turns may prompt.
struct TurnContext<'a> {
    state: &'a mut SessionState,
    runtime: &'a RuntimeConfig,
    store: &'a FsSessionStore,
    state_db: &'a SqliteStateStore,
    format: RenderFormat,
    terminal: bool,
    session: &'a mut SessionRuntime,
    /// The trust answer's echo, said once at startup where the headless loop
    /// says its other startup facts. `None` on every path that did not trust.
    trusted_echo: Option<String>,
}

/// Runs exactly one turn through the session's one per-turn entry
/// (`handle_line`), preserving the file bytes verbatim: no line splitting,
/// no trimming, no blank-line skipping. Returns the turn outcome so the
/// caller maps it to the process exit code.
fn run_single_turn(
    turn: String,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    // The one per-turn entry decides the outcome: `/exit` asks to leave
    // (exit 0), an errored turn — the stream it emitted names it — is exit
    // 5, and anything that left a mark ran to completion (exit 0). The
    // piped loop swallows these into the loop; the single-turn path maps
    // them to the process exit code instead. Both marks matter: an agent
    // turn records `turns`, while a slash turn records only `messages` —
    // the error test's unknown slash records neither, which is how the
    // outcome tells the two apart without re-parsing the turn.
    let before_turns = ctx.state.turns.len();
    let before_messages = ctx.state.messages.len();
    let should_exit = handle_line_verbatim(&turn, ctx)?;
    if should_exit {
        return Ok(TurnOutcome::Exit);
    }
    if ctx.state.turns.len() > before_turns || ctx.state.messages.len() > before_messages {
        return Ok(TurnOutcome::Completed);
    }
    Ok(TurnOutcome::Errored)
}

/// Reads lines from stdin without the rich editor, printing the status header
/// and `saya> ` marker when attached to a terminal. Used for piped input and as
/// a graceful fallback when the rich editor cannot initialize.
fn run_plain_loop(ctx: &mut TurnContext) -> Result<(), Box<dyn std::error::Error>> {
    // A startup fact the user must read once, in the loop they will see every
    // turn: a pinned root that vanished, or any other composition notice —
    // and, under bypass, the mode's activation line with its no-euphemism
    // wording and the probe/absence facts. Said as a diagnostic: the loop's
    // stdout contract is the turn stream (JSON under `--format`), and a bare
    // notice line would corrupt it.
    if let Some(notice) = ctx.session.notice() {
        eprintln!("{notice}");
    }
    if let Some(echo) = ctx.trusted_echo.as_deref() {
        eprintln!("{echo}");
    }
    if let Some(line) =
        super::session_activation::line_if_bypass(ctx.state, ctx.runtime, &ctx.session.universe())
    {
        println!("{line}");
    }
    // Bypass × unbound × non-terminal: no prompt was possible, so nothing
    // bound and the lane cannot compose either — bypass runs with the lane
    // absent, and the notice says so (the fail-closed intersection). Said
    // as a diagnostic beside the other startup facts: the turn stream owns
    // stdout, and under `--format json` a bare line would corrupt it.
    if let Some(note) = super::session_trust::bypass_no_lane_note(
        super::session_activation::is_bypass_mode(ctx.state),
        ctx.session.universe().host_composed(),
    ) {
        eprintln!("{note}");
    }
    let mut input = String::new();
    loop {
        if ctx.terminal {
            println!("{}", super::session_prompt::status_line(ctx.state));
            print!("saya> ");
            io::stdout().flush()?;
        }
        input.clear();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }
        if handle_line(&input, ctx)? {
            break;
        }
    }
    Ok(())
}

/// Processes one input line: dispatches a slash command or an agent prompt,
/// renders the resulting action, and persists the redacted session. Returns
/// `Ok(true)` when the session should exit.
fn handle_line(line: &str, ctx: &mut TurnContext) -> Result<bool, Box<dyn std::error::Error>> {
    // Piped stdin reads line by line: trim the line ending here, keep the
    // blank-line skip in the shared entry below so both paths share it.
    handle_line_verbatim(line.trim_end(), ctx)
}

/// The one per-turn entry every headless path shares: the piped loop
/// pre-trims each line (`handle_line`), while `--turn-file` passes the file
/// bytes verbatim (no trimming, no blank-line skipping). An empty turn here
/// is an errored turn — the stream names it and the caller exits non-zero —
/// never silent success.
fn handle_line_verbatim(
    line: &str,
    ctx: &mut TurnContext,
) -> Result<bool, Box<dyn std::error::Error>> {
    let state = &mut *ctx.state;
    let runtime = ctx.runtime;
    let store = ctx.store;
    let state_db = ctx.state_db;
    let format = ctx.format;
    let terminal = ctx.terminal;
    let session = &mut *ctx.session;
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
                // The real source arrives with `/mode` in the next slice.
                saya_agent::AgentMode::Build,
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
        // shared behaviour — the same parser, the session surface, and the
        // same composition the prompts state: a token the session composed
        // no capability for is refused there too, never seeded. A refused
        // scope is an error and seeds nothing; `/allow none` seeds nothing
        // and says so. Each newly seeded token is journalled once by the
        // shared behaviour; a failed journal write changes no grant and is
        // said in the message.
        let action = match super::session_grants::allow(
            &tokens,
            &session.universe().approval_facts(runtime),
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
