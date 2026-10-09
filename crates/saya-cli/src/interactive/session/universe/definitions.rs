impl SessionUniverse {
    /// The turn's definitions: the database tools' own surface plus this
    /// session's write-shaped members — advertised exactly where the mode's
    /// consent shape makes them usable: `ask` needs an approval surface to
    /// answer its asks, or an explicit grant the live policy holds that
    /// pre-answers the call; `bypass` **is** the consent (no ask, no surface —
    /// the piped-REPL demo runs on this); `read-only` and `never` can
    /// neither obtain an approval nor consult a grant. Each write-shaped
    /// call is decided through the approval engine per call.
    ///
    /// The grant arm is per family: each composed family — the workspace
    /// file family (`workspace_write`, `workspace_edit`), scratch, fetch,
    /// the runner, and the host-command lane — widens to a held grant of
    /// its own family under `ask`, the one mode whose asks a grant
    /// pre-answers (the engine resolves the call's own token to `Allow`
    /// without a surface). Availability is not authority: one token opens
    /// one family and no other argument — the advertised tool is still
    /// judged per call against its actual arguments and exact grant, and
    /// an uncovered call denies without prompting. The chart save and the
    /// ungrantable effects ride the surface alone, never a grant. The
    /// decisions live in `session_grant_surface` (workspace files) and
    /// `session_grant_families` (the rest), over the grant view the
    /// session surfaces bind per turn; an unbound view is the closed
    /// shape, and a grant recorded mid-session recomputes into the next
    /// turn's definitions through the same live store.
    ///
    /// `can_obtain_approval` means "this surface can obtain a per-call
    /// approval at all" — the line REPL's live-terminal fact, the TUI's
    /// modal (always true). It is deliberately NOT `can_prompt` ("this
    /// surface may read stdin"): the TUI must never read stdin under the
    /// alternate screen, yet obtains every approval through its modal. One
    /// flag meaning two things is what hid every write-shaped tool from the
    /// TUI under `ask`; keep them split. The stdin fallback inside
    /// `TerminalApproval::approve` reads `can_prompt` — this gate never does.
    ///
    /// `tasks_set` is the exception to the write-shaped rule, pushed on the
    /// path that survives the Plan filter: it declares `WriteSession` with
    /// no external side effect, so `read_only_permits` admits it — a
    /// read-only or Plan session that could not record what it is doing
    /// would be absurd. `never` still hides it, like everything else.
    pub(crate) fn definitions(
        &self,
        agent_mode: AgentMode,
        mode: ApprovalPolicy,
        can_obtain_approval: bool,
        allow_query_data: bool,
        has_state_store: bool,
        permit_candidate_writes: bool,
    ) -> Vec<ToolDefinition> {
        // `tasks_set` rides every policy but `never`, under both modes: the
        // Plan case is the one the tool exists for.
        let advertises_tasks = !matches!(mode, ApprovalPolicy::Never);
        let advertises = match mode {
            ApprovalPolicy::Ask => can_obtain_approval,
            ApprovalPolicy::Bypass => true,
            _ => false,
        } && agent_mode == AgentMode::Build;
        // The workspace file family's own availability: the approval-surface
        // rule above, or — new — an explicit `workspace-write` grant the
        // live policy holds under `ask` (the grant pre-answers the call the
        // mode would have asked about). Read-only, never, and Plan never
        // widen, and no other family moves. When the surface advertises
        // everything, this equals the standing rule; the chart save below
        // keeps depending on `advertises` alone, never on the grant.
        let grants = self.grants.get();
        let workspace_files =
            crate::interactive::session::permissions::grant_surface::workspace_files_available(
                agent_mode,
                mode,
                can_obtain_approval,
                grants.as_ref(),
            ) && self.workspace.is_some();
        // The remaining composed families' own availability: the same
        // shape — the standing surface rule, or (headless ask) a held
        // grant of that family — AND the member composed. One token opens
        // one family: availability is not authority, and the advertised
        // generic tool is still judged per call against its actual
        // arguments and exact grant (`SessionPolicy::resolve`), so an
        // uncovered call denies without prompting. The ungrantable chart
        // below stays on `advertises` alone; the fan-out rides the read
        // surface (`allow_query_data`) as today, widened by nothing.
        let scratch_files =
            crate::interactive::session::permissions::grant_families::scratch_available(
                agent_mode,
                mode,
                can_obtain_approval,
                grants.as_ref(),
            ) && self.scratch.is_some();
        let scratch_import_files = scratch_files && self.workspace.is_some();
        let fetch_family = crate::interactive::session::permissions::grant_families::fetch_available(
            agent_mode,
            mode,
            can_obtain_approval,
            grants.as_ref(),
        ) && self.fetch.is_some();
        let download_family = fetch_family && self.workspace.is_some();
        let runner_family =
            crate::interactive::session::permissions::grant_families::runner_available(
                agent_mode,
                mode,
                can_obtain_approval,
                grants.as_ref(),
            ) && self.runner.is_some();
        let command_family =
            crate::interactive::session::permissions::grant_families::command_available(
                agent_mode,
                mode,
                can_obtain_approval,
                grants.as_ref(),
            ) && self.host.is_some();
        // The database surface, always with the write permit off — the
        // session's own workspace_write below is the advertised one, and the
        // run-worded definition must not leak into the session's list. The
        // external-effect permit rides the same `advertises` condition as
        // every other write-shaped member: `render_chart` declares
        // `external_side_effect: true`, so read-only, never, and plan
        // enforcement deny every call — it stays hidden there, not
        // advertised-and-denied.
        let mut defs = DatabaseTools::definitions_with_chart_save(
            allow_query_data,
            has_state_store,
            permit_candidate_writes,
            false,
            advertises,
            self.workspace.is_some() && advertises,
        );
        if advertises_tasks {
            defs.push(crate::interactive::session::tasks::render::tasks_set_definition());
        }
        if workspace_files {
            defs.push(session_definitions::workspace_write());
            defs.push(session_definitions::workspace_edit());
        }
        if scratch_files {
            defs.push(session_definitions::scratch_sql());
        }
        if scratch_import_files {
            defs.push(session_definitions::scratch_import());
        }
        if fetch_family {
            defs.push(session_definitions::http_fetch());
        }
        if download_family {
            defs.push(session_definitions::http_download());
        }
        if runner_family && let Some(runner) = self.runner.as_ref() {
            defs.push(session_definitions::run_program(runner.definition.clone()));
        }
        if command_family {
            defs.push(session_definitions::run_command());
        }
        if !advertises {
            // A headless ask with only grants advertises exactly the
            // granted families' members — the grant pre-answers their
            // calls, so the advertised tools are usable — and nothing
            // else write-shaped: one token opens one family, and the
            // ungrantable chart and fan-out ride the approval surface
            // alone. Read-only and never cannot prompt and never consult
            // grants: everything write-shaped stays hidden there. Bypass
            // never lands here — its advertised tools are usable, so the
            // advertised-but-unusable anti-pattern cannot return under
            // it. `tasks_set` above is the one exception: admitted by the
            // approval engine under read-only and Plan, so advertised
            // there too.
            return defs;
        }
        defs
    }
}
