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
    /// file family (`write`, `edit`), fetch, and the host-command
    /// lane — widens to a held grant of its own family under `ask`, the one
    /// mode whose asks a grant pre-answers (the engine resolves the call's
    /// own token to `Allow` without a surface). Availability is not
    /// authority: one token opens one family and no other argument — the
    /// advertised tool is still judged per call against its actual
    /// arguments and exact grant, and an uncovered call denies without
    /// prompting. The ungrantable effects ride the surface alone, never a
    /// grant. The decisions live in
    /// `session_grant_surface` (workspace files) and
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
    /// `checklist` is the exception to the write-shaped rule, pushed on the
    /// path that survives the Plan filter: it declares `WriteSession` with
    /// no external side effect, so `read_only_permits` admits it — a
    /// read-only or Plan session that could not record what it is doing
    /// would be absurd. `never` still hides it, like everything else.
    ///
    /// `gate_open` is the plan gate's answer, and it only ever widens under
    /// `plan`: the operator approved the submitted plan, so the surface the
    /// posture closed is advertised again — the write, shell and MCP families
    /// exactly as `build` advertises them. The engine reads the same gate's
    /// cell, so an advertised write is a write the engine admits.
    ///
    /// The `shell` is the other exception, and it is the *mode's* rather than
    /// the surface's: it is advertised wherever the mode keeps a lane for it
    /// and a workspace root binds, and it rides no grant — its own lane
    /// decision is the consent (`session/universe/shell.rs`). So `read-only`
    /// advertises it (the sandbox lane, where nothing in the checkout is
    /// writable) even though the engine denies every external side effect
    /// there, and a headless `ask` advertises it where the sandbox proves,
    /// because a `sandbox: true` call needs no prompt at all. Plan hides it
    /// with the rest of the write-shaped surface, whose posture denies an
    /// external side effect outright.
    // The arguments are the turn's facts, one per decision the doc above
    // makes; bundling them would hide which surface fact each caller holds.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn definitions(
        &self,
        agent_mode: AgentMode,
        gate_open: bool,
        mode: ApprovalPolicy,
        can_obtain_approval: bool,
        allow_query_data: bool,
        has_state_store: bool,
        permit_candidate_writes: bool,
        tools: &saya_config::ResolvedTools,
    ) -> Vec<ToolDefinition> {
        // `checklist` rides every policy but `never`, under both modes: the
        // Plan case is the one the tool exists for.
        let advertises_tasks = !matches!(mode, ApprovalPolicy::Never);
        // Whether an approval is reachable at all here: the asking postures
        // need a surface that can obtain one, `bypass` is the consent itself,
        // and `read-only` and `never` can neither ask nor consult a grant.
        let can_ask = match mode {
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => can_obtain_approval,
            ApprovalPolicy::Bypass => true,
            ApprovalPolicy::ReadOnly | ApprovalPolicy::Never => false,
        };
        // The posture the surface advertises under. An open gate widens
        // exactly one posture — `plan` advertises what `build` advertises —
        // because the engine reads the same gate's cell and admits those
        // calls again. Everywhere else the posture is the one the turn stated.
        let posture = match (agent_mode, gate_open) {
            (AgentMode::Plan, true) => AgentMode::Build,
            (stated, _) => stated,
        };
        // The standing surface rule: the asking modes advertise what an
        // approval surface can serve, `bypass` advertises everything, and the
        // two no-surface modes advertise nothing here.
        let advertises = can_ask && posture == AgentMode::Build;
        // The workspace file family's own availability: the approval-surface
        // rule above, or — new — an explicit `workspace-write` grant the
        // live policy holds under `ask` (the grant pre-answers the call the
        // mode would have asked about), or the `accept-edits` mode's own
        // pre-answer, which needs no surface at all. Read-only and never
        // never widen, and no other family moves; Plan widens only through
        // its gate, which is why the families are asked about `posture` and
        // not about the turn's stated mode. When the surface advertises
        // everything, this equals the standing rule.
        let grants = self.grants.get();
        let workspace_files =
            crate::interactive::session::permissions::grant_surface::workspace_files_available(
                posture,
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
        // uncovered call denies without prompting. The ungrantable fan-out
        // rides the read surface (`allow_query_data`) as today, widened by
        // nothing.
        let fetch_family = crate::interactive::session::permissions::grant_families::fetch_available(
            posture,
            mode,
            can_obtain_approval,
            grants.as_ref(),
        ) && self.fetch.is_some();
        let download_family = fetch_family && self.workspace.is_some();
        let command_family =
            crate::interactive::session::permissions::grant_families::command_available(
                posture,
                mode,
                can_obtain_approval,
                grants.as_ref(),
            ) && self.host.is_some();
        // The session's `shell`: the shared command tool, advertised wherever
        // the mode keeps a lane for it and a
        // workspace root binds — `ask` (through this turn's approval surface,
        // or the sandbox headless), `bypass`, and `read-only` (the sandbox
        // lane, nothing writable) — and nowhere under `never`. It rides no
        // grant: a session grant names a program or a destination, and the
        // shell's own lane decision is the consent. A closed gate under Plan
        // never reaches here: the posture denies an external side effect, so
        // the shell hides with the rest of the write-shaped surface, and an
        // open gate reaches here exactly as `build` does.
        let shell_family =
            posture == AgentMode::Build && self.shell_available(mode, can_obtain_approval);
        // The database surface, always with the write permit off — the
        // session's own write below is the advertised one, and the
        // run-worded definition must not leak into the session's list.
        let mut defs = DatabaseTools::definitions(
            allow_query_data,
            has_state_store,
            permit_candidate_writes,
            false,
        );
        if advertises_tasks {
            defs.push(crate::interactive::session::tasks::render::checklist_definition());
        }
        // The gate's own tool: advertised exactly where it is the door — under
        // the `plan` posture, and only where an approval is reachable at all.
        // A `read-only` or `never` plan session is advertised no door, because
        // neither posture can obtain the approval the door leads to.
        if agent_mode == AgentMode::Plan && can_ask {
            defs.push(crate::agent::tools::coding::plan_gate::PlanGate::definition());
        }
        if workspace_files {
            defs.push(session_definitions::workspace_write());
            defs.push(session_definitions::workspace_edit());
        }
        if fetch_family {
            defs.push(session_definitions::http_fetch());
        }
        if download_family {
            defs.push(session_definitions::http_download());
        }
        if command_family {
            defs.push(session_definitions::run_command());
        }
        if shell_family {
            // The session's shell owns background runs. The start is
            // advertised with the shell; the two readers that reach a run are
            // added only after one exists, exactly as the coding root does it.
            let mut family = crate::agent::tools::shell_policy::background_definitions(
                crate::agent::tools::shell_policy::BackgroundOwner::Session,
            );
            if !self
                .background_started
                .load(std::sync::atomic::Ordering::Acquire)
            {
                family.retain(|definition| definition.name == "shell");
            }
            defs.extend(family);
        }
        // The MCP servers' tools: advertised wherever the mode keeps a lane
        // for them (`ask` and `bypass`) and a server actually started. Like
        // the shell, they ride no grant — a grant names a program or a
        // destination, and a server's tool is neither — and, like the shell,
        // their own consent rule is the decision. Under a closed Plan the
        // posture denies an external side effect, so they hide with the rest
        // of the write-shaped surface; an open gate advertises them as
        // `build` does.
        let mcp_family = posture == AgentMode::Build
            && crate::mcp::client::advertised(mode)
            && self.mcp.as_ref().is_some_and(|lane| !lane.is_empty());
        if mcp_family && let Some(lane) = &self.mcp {
            defs.extend(lane.definitions());
        }
        if !advertises {
            // A headless ask with only grants advertises exactly the
            // granted families' members — the grant pre-answers their
            // calls, so the advertised tools are usable — and nothing
            // else write-shaped: one token opens one family, and the
            // ungrantable fan-out rides the approval surface alone. Read-only and never cannot prompt and never consult
            // grants: everything write-shaped stays hidden there. Bypass
            // never lands here — its advertised tools are usable, so the
            // advertised-but-unusable anti-pattern cannot return under
            // it. `checklist` above and the `shell` pushed above are the
            // exceptions, and both are usable without a prompt: the
            // checklist because the engine admits it under read-only and
            // Plan, the shell because its sandbox lane needs no consent
            // (it is advertised headless only where that lane exists —
            // `shell_available`).
            return crate::agent::tools::narrowed(defs, tools);
        }
        crate::agent::tools::narrowed(defs, tools)
    }
}
