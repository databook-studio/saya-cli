impl SessionRuntime {
    /// Acquires the session: the state directory (0700), the lock (refusing
    /// a live holder by pid, reclaiming a stale file), and the composed
    /// universe. `fresh` distinguishes a first start — the git worktree top
    /// binds when nothing is stated — from a resume, which re-opens the
    /// recorded pin and runs unbound when the record has none. The approval
    /// policy is built once here, from the session's mode — empty by
    /// construction: a resumed session inherits no grant, from the record
    /// or from the journal, which is the audit record, never a grant source.
    /// `sessions_root` is where `sessions/<id>/` lives, so a test can claim
    /// a root of its own; production passes [`default_session_dir`].
    pub(crate) fn acquire(
        runtime: &crate::config::runtime::RuntimeConfig,
        explicit: Option<&Path>,
        fresh: bool,
        pinned_root: Option<&str>,
        id: &str,
        mode: ApprovalPolicy,
        sessions_root: &Path,
    ) -> Result<Self, String> {
        Self::acquire_inner(Acquire {
            runtime,
            explicit,
            fresh,
            pinned_root,
            id,
            mode,
            sessions_root,
            trusted: None,
        })
    }

    /// The composed acquisition behind both paths: `acquire` (no trust
    /// answer) and the startup trust prompt's answer in `session_loop`,
    /// which fills `Acquire.trusted` directly. One struct argument keeps the
    /// arity lint's budget; the seven-argument public seam above is
    /// untouched.
    pub(crate) fn acquire_inner(args: Acquire<'_>) -> Result<Self, String> {
        let Acquire {
            runtime,
            explicit,
            fresh,
            pinned_root,
            id,
            mode,
            sessions_root,
            trusted,
        } = args;
        let store = saya_store::FsSessionStore::new(sessions_root);
        // The tombstone gate precedes every filesystem claim: a deleted or
        // being-deleted id never gets a state directory back, and the lock
        // is claimed through the lifecycle owner, which re-checks the
        // tombstone once the lock is held.
        saya_harness::session_lifecycle::ensure_not_deleted(&store, id).map_err(
            |error| match error {
                saya_harness::HarnessError::SessionDeleted { .. } => {
                    format!("session {id} was deleted; it cannot be opened")
                }
                other => format!("could not check the session state: {other}"),
            },
        )?;
        let state_dir = create_state_dir(sessions_root, id)?;
        let lock = saya_harness::session_lifecycle::acquire_session_lock(&store, &state_dir, id)
            .map_err(|error| match error {
                // The same words a run's second writer reads, with the same pid.
                saya_harness::HarnessError::LockHeld { pid } => format!(
                    "session {id} is already running (pid {pid}): a session's scratch and \
                     transcript are single-writer; stop the other process or start a new session"
                ),
                saya_harness::HarnessError::SessionDeleted { .. } => {
                    format!("session {id} was deleted; it cannot be opened")
                }
                other => format!("could not claim the session lock: {other}"),
            })?;
        // Crash recovery, once per acquisition: while this lock is held the
        // process is the only owner of `<id>`, so a crash between prepare
        // and commit is repaired before anything composes — `Prepared`
        // records become `Uncertain` and orphan before-images are removed.
        // The report is ignored and errors are swallowed: a session with no
        // checkpoints directory is a cheap no-op read, and recovery must
        // never fail an acquire.
        let _ = store.recover_checkpoints(id);
        // One recovery handle per acquisition, once the lock is held: the
        // session's checkpoint store rides the same id, and the flag is
        // the opt-in default — OFF; nothing here flips it.
        let recovery = RecoveryHandle::new(sessions_root, id);
        let walk_when_unpinned = fresh;
        // The trusted directory binds exactly like an explicit
        // `--workspace` — but only where no explicit statement exists, and
        // only on a fresh start: a resume re-opens its recorded pin and never
        // re-prompts (G3: re-trust is per-process, never persisted).
        let trusted_explicit = match (fresh, explicit, trusted) {
            (true, None, Some(dir)) => Some(dir.to_path_buf()),
            _ => None,
        };
        let asked = trusted_explicit.clone();
        let effective_explicit: Option<&Path> =
            explicit.or(trusted_explicit.as_deref().map(Path::new));
        let universe = SessionUniverse::compose_with_launch_and_path(SessionComposition {
            runtime,
            explicit: effective_explicit,
            pinned_root,
            walk_when_unpinned,
            cwd: &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            state_dir: &state_dir,
            launch: None,
            path: std::env::var_os("PATH").map(|value| value.to_string_lossy().into_owned()),
            scratch: None,
            recovery: Some(recovery.clone()),
        })?;
        let explicit = explicit.map(Path::to_path_buf).or(asked);
        Ok(Self {
            universe: Arc::new(universe),
            policy: SessionPolicy::new(mode),
            policy_mode: mode,
            _lock: lock,
            explicit,
            fresh_start: fresh,
            sessions_root: sessions_root.to_path_buf(),
            state_dir: state_dir.clone(),
            journal: Arc::new(SessionJournal::open(&state_dir)),
            journal_failure: Mutex::new(None),
            question: QuestionLifecycle::Idle,
            last_question_metrics: None,
            session_metrics: crate::local_metrics::enabled().then(|| Arc::new(LocalMetrics::new())),
            recovery,
        })
    }
}
