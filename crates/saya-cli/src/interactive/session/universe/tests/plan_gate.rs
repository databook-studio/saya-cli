// ---------------------------------------------------------------------------
// the session's plan gate: the plan posture's one door
// ---------------------------------------------------------------------------

/// The plan a test submits. Short, and the same bytes the assertions read back
/// off the surface that answered it.
const SUBMITTED_PLAN: &str = "1. rewrite src/lib.rs\n2. run cargo test";

/// The session's approval surface, as the gate sees it: it records every ask it
/// is put and answers `answer`. The stand-in for the TUI's modal and the REPL's
/// terminal prompt — the gate must reach the session through the surface's own
/// decider, never through a prompt of its own.
struct Surface {
    asked: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
    answer: bool,
}

impl Surface {
    fn new(answer: bool) -> Arc<Self> {
        Arc::new(Self {
            asked: std::sync::Mutex::new(Vec::new()),
            answer,
        })
    }

    fn asked(&self) -> Vec<(String, serde_json::Value)> {
        self.asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl ApprovalDecider for Surface {
    async fn approve(
        &self,
        tool: &saya_agent::ToolDefinition,
        arguments: &serde_json::Value,
    ) -> bool {
        self.asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((tool.name.clone(), arguments.clone()));
        self.answer
    }
}

/// Submits `SUBMITTED_PLAN` through the session's gate, with `surface` bound as
/// this turn's approval surface, and hands back the gate's answer.
async fn submit(universe: &SessionUniverse, surface: &Arc<Surface>) -> serde_json::Value {
    universe.bind_plan_ask(Some(Arc::clone(surface) as Arc<dyn ApprovalDecider>));
    universe
        .plan_gate()
        .execute(
            "submit_plan",
            serde_json::json!({ "plan": SUBMITTED_PLAN }),
        )
        .await
        .expect("the gate serves a bounded plan")
}

/// Under `plan` the write family is hidden — hidden, not denied — and the
/// gate's `submit_plan` is the one door the posture keeps open.
#[test]
fn under_plan_the_write_family_is_hidden_and_submit_plan_is_advertised() {
    let project = worktree("plan-gate-closed");
    let state = temp_dir("plan-gate-closed-state");
    let universe = compose(&session_runtime(), &project, &state);
    let names = advertised(&universe, AgentMode::Plan, ApprovalPolicy::Ask, true);
    for tool in PLAN_HIDDEN_TOOLS {
        assert!(
            !names.contains(&tool.to_string()),
            "plan hides {tool}: {names:?}"
        );
    }
    assert!(
        names.contains(&"submit_plan".to_string()),
        "the gate is the door plan keeps: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The gate asks through the session's own approval surface, and the plan it
/// carries is what that surface reads: one ask, the `submit_plan` call, the
/// submitted text.
#[tokio::test]
async fn the_gate_asks_through_the_session_s_own_approval_surface() {
    let project = worktree("plan-gate-ask");
    let state = temp_dir("plan-gate-ask-state");
    let universe = compose(&session_runtime(), &project, &state);
    let surface = Surface::new(true);
    let answer = submit(&universe, &surface).await;
    assert_eq!(answer["approved"], true, "{answer}");
    let asked = surface.asked();
    assert_eq!(asked.len(), 1, "the gate asks once: {asked:?}");
    assert_eq!(asked[0].0, "submit_plan");
    assert_eq!(
        asked[0].1["plan"], SUBMITTED_PLAN,
        "the surface reads the plan it is approving: {asked:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Approving the submitted plan opens the write surface: the next turn's
/// advertisement is exactly the `build` surface plus the gate's own tool.
#[tokio::test]
async fn an_approved_plan_advertises_the_write_family() {
    let project = worktree("plan-gate-open");
    let state = temp_dir("plan-gate-open-state");
    let universe = compose(&session_runtime(), &project, &state);
    let surface = Surface::new(true);
    assert_eq!(submit(&universe, &surface).await["approved"], true);
    let open: std::collections::BTreeSet<String> =
        advertised(&universe, AgentMode::Plan, ApprovalPolicy::Ask, true)
            .into_iter()
            .collect();
    let mut expected: std::collections::BTreeSet<String> =
        advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true)
            .into_iter()
            .collect();
    expected.insert("submit_plan".to_string());
    assert_eq!(
        open, expected,
        "the approval opens exactly the surface the plan posture closed"
    );
    for tool in ["write", "edit", "http_fetch", "http_download"] {
        assert!(
            open.contains(tool),
            "the open gate advertises {tool}: {open:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A refusal leaves the surface closed: the write family stays hidden, and the
/// gate stays shut.
#[tokio::test]
async fn a_refused_plan_keeps_the_write_family_hidden() {
    let project = worktree("plan-gate-refused");
    let state = temp_dir("plan-gate-refused-state");
    let universe = compose(&session_runtime(), &project, &state);
    let surface = Surface::new(false);
    let answer = submit(&universe, &surface).await;
    assert_eq!(answer["approved"], false, "{answer}");
    assert!(!universe.plan_gate_open(), "a refusal opens nothing");
    let names = advertised(&universe, AgentMode::Plan, ApprovalPolicy::Ask, true);
    for tool in PLAN_HIDDEN_TOOLS {
        assert!(
            !names.contains(&tool.to_string()),
            "the refused plan still hides {tool}: {names:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The session's dispatch door serves `submit_plan` from the gate, beside the
/// shell and the MCP lane: the advertised name is a name the executor answers,
/// and the call it serves is the gate's own.
#[tokio::test]
async fn the_session_executor_routes_submit_plan_to_the_gate() {
    let project = worktree("plan-gate-exec");
    let state = temp_dir("plan-gate-exec-state");
    let runtime = session_runtime();
    let universe = compose(&runtime, &project, &state);
    let surface = Surface::new(true);
    universe.bind_plan_ask(Some(Arc::clone(&surface) as Arc<dyn ApprovalDecider>));
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(
        Arc::clone(&database),
        &CancellationToken::new(),
        None,
        None,
        Some(universe.plan_gate()),
        runtime.resolved.tools.clone(),
    );
    let answer = executor
        .execute(
            "submit_plan",
            serde_json::json!({ "plan": SUBMITTED_PLAN }),
        )
        .await
        .expect("the gate serves the call the list advertises");
    assert_eq!(answer["approved"], true, "{answer}");
    assert!(
        universe.plan_gate_open(),
        "the routed call is the gate's own, not a second implementation"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
