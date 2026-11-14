impl SessionUniverse {
    /// A universe with nothing in it — the struct-literal test support for
    /// the TUI's idle apps, which never dispatch a tool. Production composes
    /// through [`SessionUniverse::compose`] only.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        let (plan, plan_ask) = plan_gate_pair();
        Self {
            workspace: None,
            fetch: None,
            host: None,
            deny: crate::interactive::session::permissions::deny::SessionDeny::default(),
            tasks: crate::interactive::session::tasks::SessionTasks::default(),
            primary: crate::grant_token::TurnPrimary::default(),
            grants: crate::interactive::session::permissions::grant_surface::GrantView::default(),
            notice: None,
            recovery: None,
            mcp: None,
            learning_breaker: LearningBreaker::new(),
            shell_sandboxes: None,
            shell_rules: CommandRules::default(),
            file_sources: crate::agent::file_sources::FileSources::NoData,
            shell_host: saya_harness::runner::HostPolicy::default(),
            shell_background: Arc::new(saya_harness::runner::BackgroundRuns::default()),
            shell_background_cancellation: CancellationToken::new(),
            background_started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            plan,
            plan_ask,
            session_hooks: crate::agent::hooks::SessionHooks::new(),
        }
    }

    /// Composes the universe once per session process: the workspace binding
    /// (explicit `--workspace`, or the recorded pin on a resume, or the git
    /// worktree top on a fresh start) and the session fetch wiring. The host
    /// lane stays unstated here — off by construction; the launch path
    /// composes through `compose_with_launch`.
    /// Production composes through [`Self::compose_with_launch_and_path`]
    /// only, because a live acquisition must carry its recovery handle;
    /// these wrappers — which cannot — are the test fixtures' seam.
    #[cfg(test)]
    pub(crate) fn compose(
        runtime: &crate::config::runtime::RuntimeConfig,
        explicit: Option<&Path>,
        pinned_root: Option<&str>,
        walk_when_unpinned: bool,
        cwd: &Path,
    ) -> Result<Self, String> {
        Self::compose_with_launch(runtime, explicit, pinned_root, walk_when_unpinned, cwd, None)
    }

    /// Composes with the launch's host-command statement: the `--allow
    /// command:<x>` seeds (which still seed the grant), the `--deny`
    /// refusals, and the user-layer `[host_commands]` shaping. The lane
    /// itself composes wherever a workspace root binds — a seed only seeds and
    /// implies no composition. `None` means no launch statement,
    /// and the lane still composes where a root binds. Test-only, like
    /// [`Self::compose`]: a live acquisition carries its recovery handle
    /// through `compose_with_launch_and_path` directly.
    #[cfg(test)]
    pub(crate) fn compose_with_launch(
        runtime: &crate::config::runtime::RuntimeConfig,
        explicit: Option<&Path>,
        pinned_root: Option<&str>,
        walk_when_unpinned: bool,
        cwd: &Path,
        launch: Option<&session_host::HostLaunch>,
    ) -> Result<Self, String> {
        Self::compose_with_launch_and_path(SessionComposition {
            runtime,
            explicit,
            pinned_root,
            walk_when_unpinned,
            cwd,
            launch,
            path: std::env::var_os("PATH").map(|value| value.to_string_lossy().into_owned()),
            recovery: None,
            mcp: None,
        })
    }

    /// The full composer with the PATH seam explicit: `None` is the no-PATH
    /// environment. Production reads the environment once at the seam above;
    /// the no-PATH property pins this struct-carrying function with
    /// `path: None` directly, so no test mutates the process environment.
    /// One struct argument keeps the arity lint's budget (the struct is one
    /// parameter); the seven-argument public seams above are untouched.
    pub(crate) fn compose_with_launch_and_path(
        composition: SessionComposition<'_>,
    ) -> Result<Self, String> {
        let SessionComposition {
            runtime,
            explicit,
            pinned_root,
            walk_when_unpinned,
            cwd,
            launch,
            path,
            recovery,
            mcp,
        } = composition;
        // The deny list, once per session: the launch's `--deny` refusals
        // plus the user-layer `[session_commands] deny`. Launch-only — a
        // mid-session deny over a held grant would leave a journaled token
        // that gates nothing — so deny precedes every grant by construction.
        // Deny wins at the door: it is consulted before any grant, so a
        // denied name is refused in every mode, bypass included.
        let mut deny_names: Vec<String> =
            launch.map(|launch| launch.deny_list()).unwrap_or_default();
        deny_names.extend(runtime.resolved.session_deny.programs.iter().cloned());
        let deny =
            crate::interactive::session::permissions::deny::SessionDeny::from_names(deny_names)?;
        let (workspace, notice) = bind_from_pins(explicit, pinned_root, walk_when_unpinned, cwd)?;
        // The checkout's data-file route, detected once at session start —
        // before `path` moves into the host lane. The prompt adds the block
        // only where the turn also advertises `shell` (`turn.rs`).
        let file_sources = crate::agent::file_sources::detect(
            workspace.as_ref().map(|bound| bound.workspace.as_ref()),
            path.as_deref().map(std::ffi::OsStr::new),
        );
        // Fetch: the session-wide policy — HTTPS only, refused ranges still
        // refused, every host consented per call by the approval engine.
        let transport = ReqwestTransport::new()
            .map_err(|error| format!("the fetch transport could not be built: {error}"))?;
        let fetch = Arc::new(FetchTools::new(
            FetchPolicy::session(),
            Arc::new(transport),
            FetchLimits::for_tool_lane(),
            DownloadLimits::default(),
            workspace.as_ref().map(|bound| Arc::clone(&bound.workspace)),
            DownloadBudget::default(),
        ));
        // The host lane, once per session: wherever a workspace root binds
        // — no root, no lane, structural, because the child's cwd is pinned
        // to the root. The child's PATH is the parent's own. With no root
        // bound the lane composes nothing without touching the environment.
        // A root-bound session with no PATH in the environment composes the
        // lane away with the no-PATH notice instead of refusing the
        // session: a missing PATH disables an optional lane, never startup.
        let root = workspace.as_ref().map(|bound| bound.root.as_path());
        let unstated;
        let lane = match launch {
            Some(lane) => lane,
            None => {
                unstated = session_host::HostLaunch::unstated(runtime);
                &unstated
            }
        };
        let (host, no_path_notice) = match root {
            Some(_) => session_host::compose_host_lane(lane, root, path)?,
            None => (session_host::compose_host(lane, None, String::new())?, None),
        };
        // The startup notice names the exceptional shapes: a vanished pin,
        // a missing PATH on a root-bound session, or an unavailable Windows
        // host lane. The lane facts may coincide, so retain each rather
        // than hiding one behind another. A composed lane is not
        // exceptional — the lane composes wherever a root binds, and the
        // status header's `host:` segment (`host:unsandboxed` / `host:off`)
        // already carries the fact — so it rides no notice. The per-call
        // ask under `ask`, and the bypass activation line's lane fact under
        // `bypass`, carry the consent surfaces; there is no launch notice
        // to emit for a stated frame.
        // The MCP servers' own reports join the same notice: a server that
        // could not be started, or a tool this client refused to offer, is a
        // startup fact the user must read once — its absence from the tool
        // list would otherwise look like a server with nothing to offer.
        let mcp_notice = mcp
            .as_ref()
            .map(|lane| lane.reports().join("\n"))
            .filter(|reports| !reports.is_empty());
        // The default coding root edits a Git worktree in place: a fresh
        // session names that, and the rewind remedy, once at startup.
        let coding_notice = (walk_when_unpinned
            && runtime.resolved.coding.root_mode == saya_config::JobRootMode::InPlace)
            .then_some(workspace.as_ref())
            .flatten()
            .and_then(|bound| {
                crate::interactive::session_workspace::coding_in_place_notice(&bound.root)
            });
        let notice = [notice, no_path_notice, mcp_notice, coding_notice]
            .into_iter()
            .flatten()
            .reduce(|mut notices, notice| {
                notices.push('\n');
                notices.push_str(&notice);
                notices
            });
        // The shell's two session-stable facts: the root its sandboxes are
        // proven against (only where a root binds — no root, no shell), and
        // the host-lane policy a host call runs under. Neither proves a
        // sandbox: the probe spawns canaries, so it runs lazily, once per
        // shape, on the first turn that needs one (`shell.rs`).
        let shell_sandboxes = root.map(|root| ShellSandboxes::new(root.to_path_buf()));
        let shell_host = crate::agent::tools::shell_policy::host_policy(runtime);
        let shell_rules = runtime.resolved.permissions.rules.clone();
        // The plan gate, once per session: the mode can enter `plan` at any
        // point, and the gate the operator's approval opens is the one the
        // switch closes.
        let (plan, plan_ask) = plan_gate_pair();
        Ok(Self {
            workspace,
            fetch: Some(fetch),
            host,
            deny,
            tasks: crate::interactive::session::tasks::SessionTasks::default(),
            primary: crate::grant_token::TurnPrimary::default(),
            grants: crate::interactive::session::permissions::grant_surface::GrantView::default(),
            notice,
            recovery,
            mcp,
            learning_breaker: LearningBreaker::new(),
            shell_sandboxes,
            shell_rules,
            file_sources,
            shell_host,
            shell_background: Arc::new(saya_harness::runner::BackgroundRuns::default()),
            shell_background_cancellation: CancellationToken::new(),
            background_started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            plan,
            plan_ask,
            session_hooks: crate::agent::hooks::SessionHooks::new(),
        })
    }
}

impl Drop for SessionUniverse {
    /// The session is ending: every background run it owns is cancelled and
    /// its whole process group killed, whatever turn composed the shell that
    /// started it. The registry's own drop does the same when its last owner
    /// goes; this is the prompt path, and it also covers a shell still held
    /// elsewhere at session end.
    fn drop(&mut self) {
        self.shell_background_cancellation.cancel();
    }
}
