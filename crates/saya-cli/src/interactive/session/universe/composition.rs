impl SessionUniverse {
    /// A universe with nothing in it — the struct-literal test support for
    /// the TUI's idle apps, which never dispatch a tool. Production composes
    /// through [`SessionUniverse::compose`] only.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            workspace: None,
            scratch: None,
            fetch: None,
            runner: None,
            host: None,
            host_ran: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            deny: crate::interactive::session::permissions::deny::SessionDeny::default(),
            tasks: crate::interactive::session::tasks::SessionTasks::default(),
            primary: crate::grant_token::TurnPrimary::default(),
            grants: crate::interactive::session::permissions::grant_surface::GrantView::default(),
            notice: None,
            recovery: None,
            probe_refused: false,
            learning_breaker: LearningBreaker::new(),
        }
    }

    /// Composes the universe once per session process: the workspace binding
    /// (explicit `--workspace`, or the recorded pin on a resume, or the git
    /// worktree top on a fresh start), the scratch database over the
    /// session's state directory, the session fetch wiring, and the runner
    /// where the host proves. The host lane stays unstated here — off by
    /// construction; the launch path composes through `compose_with_launch`.
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
        state_dir: &Path,
    ) -> Result<Self, String> {
        Self::compose_with_launch(
            runtime,
            explicit,
            pinned_root,
            walk_when_unpinned,
            cwd,
            state_dir,
            None,
        )
    }

    /// Composes with the launch's host-command statement: the `--allow
    /// command:<x>` seeds (which still seed the grant), the `--deny`
    /// refusals, and the user-layer `[host_commands]` shaping. The lane
    /// itself composes wherever a workspace root binds — a seed only seeds,
    /// it no longer implies composition. `None` means no launch statement,
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
        state_dir: &Path,
        launch: Option<&session_host::HostLaunch>,
    ) -> Result<Self, String> {
        Self::compose_with_launch_and_path(SessionComposition {
            runtime,
            explicit,
            pinned_root,
            walk_when_unpinned,
            cwd,
            state_dir,
            launch,
            path: std::env::var_os("PATH").map(|value| value.to_string_lossy().into_owned()),
            scratch: None,
            recovery: None,
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
            state_dir,
            launch,
            path,
            scratch,
            recovery,
        } = composition;
        // The deny list, once per session: the launch's `--deny` refusals
        // plus the user-layer `[session_commands] deny`. Launch-only — a
        // mid-session deny over a held grant would leave a journaled token
        // that gates nothing — so deny precedes every grant by construction.
        // `[jobs.runner] allow` ∩ deny is not an error: the allowlist also
        // governs runs, where deny does not ride; in a session deny wins at
        // the door.
        let mut deny_names: Vec<String> =
            launch.map(|launch| launch.deny_list()).unwrap_or_default();
        deny_names.extend(runtime.resolved.session_deny.programs.iter().cloned());
        let deny =
            crate::interactive::session::permissions::deny::SessionDeny::from_names(deny_names)?;
        let (workspace, notice) = bind_from_pins(explicit, pinned_root, walk_when_unpinned, cwd)?;
        // Scratch: one DuckDB file per session state directory, the pinned
        // scratch semantics (external access off, 0600) either way. It is
        // engine state — a file tool cannot reach it and neither can a child.
        let scratch = match scratch {
            Some(scratch) => scratch.as_ref().clone(),
            None => ScratchSql::open(state_dir).map_err(|error| {
                format!("the session scratch database could not be opened: {error}")
            })?,
        };
        let scratch =
            scratch.rebind_workspace(workspace.as_ref().map(|bound| Arc::clone(&bound.workspace)));
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
        let mut runner_composed = None;
        let mut runner_notice = None;
        if let Some(bound) = workspace.as_ref() {
            let composition = compose_runner(runtime, &bound.root, state_dir)?;
            runner_notice = composition.probe_notice;
            runner_composed = composition.runner;
        }
        let probe_refused = runner_notice.is_some();
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
        // a refused probe, a missing PATH on a root-bound session, or an
        // unavailable Windows host lane. The lane facts may coincide, so
        // retain each rather than hiding one behind another.
        // composed lane is not exceptional — the lane composes wherever a
        // root binds, and the status header's `host:` segment
        // (`host:unsandboxed` / `host:off`) already carries the fact — so
        // it rides no notice. The per-call ask under `ask`, and the bypass
        // activation line's lane fact under `bypass`, carry the consent
        // surfaces; there is no launch notice to emit for a stated frame.
        let notice = [notice, no_path_notice, runner_notice]
            .into_iter()
            .flatten()
            .reduce(|mut notices, notice| {
                notices.push('\n');
                notices.push_str(&notice);
                notices
            });
        Ok(Self {
            workspace,
            scratch: Some(Arc::new(scratch)),
            fetch: Some(fetch),
            runner: runner_composed,
            host,
            host_ran: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            deny,
            tasks: crate::interactive::session::tasks::SessionTasks::default(),
            primary: crate::grant_token::TurnPrimary::default(),
            grants: crate::interactive::session::permissions::grant_surface::GrantView::default(),
            notice,
            recovery,
            probe_refused,
            learning_breaker: LearningBreaker::new(),
        })
    }
}
