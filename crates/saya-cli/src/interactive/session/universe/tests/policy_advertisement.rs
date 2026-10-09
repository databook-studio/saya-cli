/// Build is unchanged: for each of the four approval policies the Build
/// definition list is exactly what `release/0.4.1` produces — except
/// read-only and never no longer advertise `render_chart`: the tool declares
/// `external_side_effect: true`, so their enforcement denies every call,
/// and advertisement now agrees — plus `tasks_set`, the session-metadata
/// write every policy but `never` carries. Pinned by name on the plain
/// worktree composition (no runner composed, so no `run_program`; the
/// unstated host lane composes, so `run_command` rides).
///
/// The flag is the approval-surface flag: `ask` with an approval surface
/// advertises the write-shaped set; `bypass` needs no surface; `read-only`
/// and `never` hide whatever the surface. The `(Ask, false)` row — a
/// surface that can obtain no approval — keeps the anti-pattern pinned.
#[cfg(not(windows))]
#[test]
fn build_advertisement_is_pinned_for_every_approval_policy() {
    let project = worktree("build-pinned");
    let state = temp_dir("build-pinned-state");
    let universe = compose(&session_runtime(None), &project, &state);
    for (mode, can_obtain_approval, names) in [
        (
            ApprovalPolicy::Ask,
            false,
            vec![
                "schema_discovery",
                "request_clarification",
                "workspace_read",
                "workspace_list",
                "glob",
                "grep",
                "bounded_sql_query",
                "bounded_sql_query_all",
                "result_shape",
                "column_health",
                "join_check",
                "designate_answer",
                "tasks_set",
            ],
        ),
        (
            ApprovalPolicy::Ask,
            true,
            vec![
                "schema_discovery",
                "request_clarification",
                "workspace_read",
                "workspace_list",
                "glob",
                "grep",
                "bounded_sql_query",
                "bounded_sql_query_all",
                "result_shape",
                "column_health",
                "join_check",
                "render_chart",
                "designate_answer",
                "tasks_set",
                "workspace_write",
                "workspace_edit",
                "scratch_sql",
                "scratch_import",
                "http_fetch",
                "http_download",
                "run_command",
            ],
        ),
        (
            ApprovalPolicy::Bypass,
            false,
            vec![
                "schema_discovery",
                "request_clarification",
                "workspace_read",
                "workspace_list",
                "glob",
                "grep",
                "bounded_sql_query",
                "bounded_sql_query_all",
                "result_shape",
                "column_health",
                "join_check",
                "render_chart",
                "designate_answer",
                "tasks_set",
                "workspace_write",
                "workspace_edit",
                "scratch_sql",
                "scratch_import",
                "http_fetch",
                "http_download",
                "run_command",
            ],
        ),
        (
            ApprovalPolicy::ReadOnly,
            true,
            vec![
                "schema_discovery",
                "request_clarification",
                "workspace_read",
                "workspace_list",
                "glob",
                "grep",
                "bounded_sql_query",
                "bounded_sql_query_all",
                "result_shape",
                "column_health",
                "join_check",
                "designate_answer",
                "tasks_set",
            ],
        ),
        (
            ApprovalPolicy::Never,
            true,
            vec![
                "schema_discovery",
                "request_clarification",
                "workspace_read",
                "workspace_list",
                "glob",
                "grep",
                "bounded_sql_query",
                "bounded_sql_query_all",
                "result_shape",
                "column_health",
                "join_check",
                "designate_answer",
            ],
        ),
    ] {
        assert_eq!(
            advertised(&universe, AgentMode::Build, mode, can_obtain_approval),
            names
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<String>>(),
            "Build under {mode:?} pins the advertised names"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// `render_chart` is advertised exactly where the engine would not deny it:
/// ask-with-prompt or bypass under Build. For each of the four approval
/// policies × {Build, Plan}, the advertised set contains the chart iff a
/// `SessionPolicy` under that posture does not deny its effect. Tool names
/// are pinned on the chart's presence so a future drift fails loudly.
#[test]
fn render_chart_advertisement_agrees_with_enforcement_for_every_policy_and_mode() {
    use saya_agent::{ApprovalDecision, SessionPolicy, ToolEffect};
    let project = worktree("chart-advert");
    let state = temp_dir("chart-advert-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let chart_effect = ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: true,
        local_state: saya_agent::LocalStateEffect::None,
    };
    for (agent_mode, policy, can_obtain_approval) in [
        (AgentMode::Build, ApprovalPolicy::Ask, true),
        (AgentMode::Build, ApprovalPolicy::Ask, false),
        (AgentMode::Build, ApprovalPolicy::Bypass, false),
        (AgentMode::Build, ApprovalPolicy::ReadOnly, true),
        (AgentMode::Build, ApprovalPolicy::Never, true),
        (AgentMode::Plan, ApprovalPolicy::Ask, true),
        (AgentMode::Plan, ApprovalPolicy::Bypass, false),
        (AgentMode::Plan, ApprovalPolicy::ReadOnly, true),
        (AgentMode::Plan, ApprovalPolicy::Never, true),
    ] {
        let definitions =
            universe.definitions(agent_mode, policy, can_obtain_approval, true, false, false);
        let advertised_chart = definitions
            .iter()
            .any(|definition| definition.name == "render_chart");
        // Denied means the engine denies — or the decider would with no
        // approval surface to answer: under ask without one every
        // approval-gated tool (the stdin fallback's `Ask if !can_prompt
        // => false` is the same shape: an unanswerable ask denies) is
        // refused, which is exactly why the write-shaped members hide
        // there too.
        let denied = matches!(
            SessionPolicy::new(policy)
                .with_agent_mode(agent_mode)
                .resolve(&chart_effect, None),
            ApprovalDecision::Deny { .. }
        ) || (policy == ApprovalPolicy::Ask && !can_obtain_approval);
        assert_eq!(
            advertised_chart, !denied,
            "{agent_mode:?} under {policy:?} (can_obtain_approval={can_obtain_approval}): advertised={advertised_chart} but denied={denied}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The normal interactive case does not regress: ask with a prompt surface
/// under Build still advertises `render_chart` alongside the chart-less SQL
/// family, in the same position it always held.
#[test]
fn ask_with_prompt_under_build_still_advertises_render_chart() {
    let project = worktree("chart-ask");
    let state = temp_dir("chart-ask-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    let position = names
        .iter()
        .position(|name| name == "render_chart")
        .expect("ask-with-prompt under Build advertises render_chart");
    assert_eq!(
        &names[position - 1..position + 2],
        &["join_check", "render_chart", "designate_answer"],
        "render_chart keeps its place in the normal interactive tool list: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
