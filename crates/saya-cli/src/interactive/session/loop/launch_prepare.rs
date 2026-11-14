fn prepare_launch(cli: Cli) -> Result<LaunchContext, Box<dyn std::error::Error>> {
    crate::app::refuse_conflicting_consent(
        cli.options.permission_mode.as_deref(),
        cli.options.approval_mode.as_deref(),
    )?;
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
    // The startup trust decision: one decision — prompt
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
        && crate::interactive::session::permissions::trust::should_prompt(
            &crate::interactive::session::permissions::trust::TrustPromptContext {
                is_terminal: true,
                fresh,
                has_explicit: cli.options.workspace.is_some(),
                has_pin: state.workspace_root.is_some(),
                root_bound: state.workspace_root.is_some()
                    || crate::interactive::session_workspace::resolve_root(
                        None,
                        &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                    )
                    .ok()
                    .flatten()
                    .is_some(),
                turn_file: cli.options.turn_file.is_some(),
            },
        );
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
    // A stated `--permission-mode` is this process's own mode for the
    // session, and it overrides whatever the fresh defaults or a resumed
    // record carried. It moves the same pair `/mode` moves, so the flag, the
    // command and the key cycle cannot land on different postures. Read here,
    // before the engine is acquired, so the session's one policy is built
    // from the mode the launch stated rather than being switched into it
    // afterwards.
    let allow_bypass = config::runtime::allow_bypass(&runtime);
    if let Some(spelling) = cli.options.permission_mode.as_deref() {
        let mode = crate::interactive::session::permissions::mode::stated_mode(
            spelling,
            allow_bypass,
        )
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let (consent, agent) = mode.decompose();
        if let Some(consent) = consent {
            state.approval_mode =
                crate::interactive::session::commands::approval_name(consent);
        }
        state.agent_mode = agent.as_str().into();
    }
    state.bind_runtime_endpoint(runtime.resolved.ai.base_url.as_deref());
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
    // workspace binding, fetch, runner, host lane — plus the
    // session's one approval policy, built from the session's mode. Held for
    // the process; the lock releases when it drops.
    //
    // The host lane composes here, once per session: the launch statement
    // (the `--allow` seeds of every shape — `command:<x>` included, which
    // seed the grant, never imply composition — the `--deny` refusals, the
    // user-layer config) is read, the universe composes with it, and the
    // seeds land in the grant store below, before anything runs. A seed
    // the composition cannot carry is a launch usage error — never a
    // silently dropped token. A resumed session restarts unstated: grants
    // die with the process, and the launch statement belonged to the
    // previous process.
    let launch = crate::interactive::session_host::HostLaunch::from_options(&cli.options, &runtime);
    // The launch contradiction: `--allow command:x` together with `--deny
    // x` grants what it refuses — an exit-2 usage error, before anything
    // exists. Every `--deny` entry is a bare name first: a path-shaped,
    // traversal, prefix, or glob entry is a typed launch error.
    for entry in &cli.options.deny {
        if let Err(error) =
            crate::interactive::session::permissions::deny::validate_deny_entry(entry)
        {
            return Err(format!("invalid --deny entry: {error}").into());
        }
    }
    for seed in &cli.options.allow {
        if let Some(name) = seed.strip_prefix("command:")
            && cli.options.deny.iter().any(|denied| denied == name)
        {
            return Err(
                crate::interactive::session::permissions::deny::launch_contradiction(name).into(),
            );
        }
    }
    let mut session =
        SessionRuntime::acquire_inner(crate::interactive::session::runtime::Acquire {
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
        trusted_echo = session
            .root()
            .map(crate::interactive::session::permissions::trust::trusted_root_line);
    }
    // Recompose with the launch statement on a fresh start: `acquire`
    // composed without the launch's deny refusals. The deny list rides the
    // same recomposition — refusal-only, composes nothing — so a deny-only
    // launch still composes its refusals. The lane itself composes wherever
    // a root binds regardless, through the same composer — one composer
    // behind both paths — so every fresh start recomposes.
    if fresh {
        session.recompose_with_launch(&runtime, state.workspace_root.as_deref(), Some(&launch))?;
    }
    // The session's own start hook, once: the launch is the session start,
    // and the bounded stdout is carried to every turn's gate. A TUI session
    // whose root is still being trust-answered has no root yet, so it runs
    // nothing here; its first turn starts the hooks once the root binds.
    session.start_hooks(&runtime);
    // The launch posture, once per process, for every launch kind — fresh,
    // `--continue`, `--resume`: the header facts derive from the universe the
    // launch composed, and the deny list journals on a fresh start only. The
    // derivation is unconditional (a resumed launch's universe composes the
    // lane wherever a root binds, so its header must say so); the deny
    // JOURNAL write stays fresh-only — the launch statement belonged to the
    // previous process, whose start journaled its own list.
    derive_launch_posture(&mut state, &session, fresh);
    // The permission posture, once per process, for every launch kind. The
    // bypass gate is read from its one source and carried on the state, so
    // the switch, the key cycle and the bar read the same answer. A record
    // that says `bypass` while this process withholds it degrades to `ask`:
    // the record cannot grant a consent the configuration refuses, and a
    // narrowing records no activation. The task posture then rides the
    // session's one policy from the first turn, so a resumed Plan session
    // refuses writes at the engine and not only in the advertisement.
    state.allow_bypass = allow_bypass;
    let (approval, agent) =
        crate::interactive::session::permissions::mode::launch_posture(
            &state.approval_mode,
            &state.agent_mode,
            cli.options.approval_mode.is_some(),
            state.allow_bypass,
        );
    state.approval_mode = approval;
    state.agent_mode = agent;
    session.switch_policy(None, state.agent_mode_parsed());
    if fresh && !cli.options.allow.is_empty() {
        // Every stated `--allow` token — `command:` included — goes through
        // the one grammar-then-composition gate `/allow` uses
        // (`seed_launch_allow`): the grammar is the only authority a token
        // parses under, and a token this session's composition cannot
        // carry is a launch usage error with its own reason, never
        // silently dropped. Pass `cli.options.allow` directly so every
        // scope type reaches both checks.
        let seeded = crate::interactive::session::permissions::grants::seed_launch_allow(
            &cli.options.allow,
            &session.universe().approval_facts(&runtime),
            session.policy().grants(),
        )
        .map_err(|error| format!("invalid --allow seed: {error}"))?;
        for token in seeded {
            if let Err(error) = session
                .journal()
                .granted(&token, saya_store::GrantSource::Seed)
            {
                eprintln!(
                    "{}",
                    crate::interactive::session::permissions::grants::journal_warning(&error)
                );
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
    // The user-authored slash commands, read once per process: the user's own
    // `commands/` directory under the config home, then the workspace's
    // `.saya/commands/`. Loading is read-only and grants nothing — a command
    // file can only ever expand to a user message (`slash::user_commands`),
    // so the untrusted project directory needs no further gate. The
    // diagnostics (a refused file, an ignored front-matter key) are said by
    // each surface at startup.
    state.user_commands = crate::slash::user_commands::load(
        &crate::config::sources::user_config_dir(),
        session
            .root()
            .map(|root| root.join(".saya").join("commands"))
            .as_deref(),
    );
    // The launch stated the mode; a fresh session under bypass — or a
    // resume whose `--approval-mode` explicitly overrode the record — is an
    // activation, and the journal records it here, before anything runs. A
    // resume that merely carries the persisted mode re-prints the line but
    // consents to nothing new, so nothing is journalled.
    if crate::interactive::session::permissions::activation::bypass_activated_at_launch(
        fresh,
        cli.options.approval_mode.is_some(),
        &state.approval_mode,
    ) {
        session.journal_bypass_activation(saya_store::BypassSource::Launch);
    }
    Ok(LaunchContext {
        cli,
        runtime,
        store,
        state_db,
        format,
        state,
        session,
        fresh,
        terminal_probe,
        trusted_echo,
        tui_trust_pending,
        launch,
    })
}
