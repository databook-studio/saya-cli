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

/// The launch posture, once per process, for every launch kind — fresh,
/// `--continue`, `--resume`. The status header's facts — the lane bit and the
/// deny list — copy from the universe this process composed, so every prompt
/// carries the `host:` segment the session will actually run under. The lane
/// composes wherever a root binds through the one composer behind both paths
/// (`acquire` composed a resumed launch's universe from the recorded pin; a
/// fresh start recomposed with the launch statement), so the derivation is
/// unconditional: guarded to fresh starts, a resumed session's header stayed
/// on the `#[serde(skip)]` defaults (`host:off`, no deny list) while its
/// universe advertised and executed the lane — only the TUI-exit site
/// corrected it after the fact.
///
/// The deny JOURNAL write keeps its fresh-only guard, a separate statement
/// from the posture copy: a resumed session restarts unstated (the launch
/// statement belonged to the previous process, whose start journaled its own
/// list), so nothing re-journals here.
pub(crate) fn derive_launch_posture(
    state: &mut SessionState,
    session: &SessionRuntime,
    fresh: bool,
) {
    let universe = session.universe();
    state.sync_host_posture(&universe);
    let denied = universe.deny_programs();
    if fresh
        && !denied.is_empty()
        && let Err(error) = session.journal().deny_list(&denied)
    {
        eprintln!(
            "{}",
            crate::interactive::session::permissions::grants::journal_warning(&error)
        );
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
    let already_binds = crate::interactive::session_workspace::resolve_root(None, &cwd)
        .ok()
        .flatten()
        .is_some();
    if already_binds {
        return None;
    }
    let ctx = crate::interactive::session::permissions::trust::TrustPromptContext {
        is_terminal,
        fresh,
        has_explicit: false,
        has_pin: false,
        root_bound: false,
        turn_file: false,
    };
    if !crate::interactive::session::permissions::trust::should_prompt(&ctx) {
        return None;
    }
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stderr = io::stderr();
    let answer =
        crate::interactive::session::permissions::trust::ask_trust(&mut input, &mut stderr)
            .unwrap_or(
                crate::interactive::session::permissions::trust::TrustAnswer::ContinueUnbound,
            );
    match answer {
        crate::interactive::session::permissions::trust::TrustAnswer::TrustCwd => Some(
            crate::interactive::session::permissions::trust::resolve_trusted_dir(&cwd)
                .unwrap_or(cwd),
        ),
        crate::interactive::session::permissions::trust::TrustAnswer::Workspace(dir) => Some(dir),
        crate::interactive::session::permissions::trust::TrustAnswer::ContinueUnbound => None,
    }
}
