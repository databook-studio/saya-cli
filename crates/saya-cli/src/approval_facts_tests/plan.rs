/// The plan the operator is asked to approve is the plan they read: the gate's
/// ask renders the submitted text itself, not a tool name and a promise.
#[test]
fn a_submit_plan_ask_shows_the_plan_it_approves() {
    let tool = crate::agent::tools::coding::plan_gate::PlanGate::definition();
    let plan = "1. rewrite src/lib.rs\n2. run cargo test";
    let arguments = serde_json::json!({ "plan": plan });
    let prompt = approval_prompt(&tool, &arguments, None, &session_facts(), None, None);
    assert!(
        prompt.contains(plan),
        "the prompt carries the plan's own bytes: {prompt}"
    );
    assert!(
        prompt.contains("opens the write surface"),
        "the prompt states what approving does: {prompt}"
    );
    assert!(
        prompt.contains(&format!(
            "≤ {} characters",
            crate::agent::tools::coding::plan_gate::MAX_PLAN_CHARS
        )),
        "the prompt states the bound the gate enforces: {prompt}"
    );
}

/// A plan is model-authored text painted on an operator's screen, so it is
/// sanitised exactly as a shell command is: a CR or an ESC that could rewrite
/// the line being read, and an invisible format character that could hide part
/// of what is being approved, both become the replacement character — while a
/// newline stays a newline, because the plan's shape is part of what is read.
#[test]
fn a_submit_plan_ask_sanitises_the_plan() {
    let tool = crate::agent::tools::coding::plan_gate::PlanGate::definition();
    let arguments =
        serde_json::json!({ "plan": "step one\rstep two\u{1b}[31mred\u{202e}hidden\nstep three" });
    let prompt = approval_prompt(&tool, &arguments, None, &session_facts(), None, None);
    for smuggled in ['\r', '\u{1b}', '\u{202e}'] {
        assert!(
            !prompt.contains(smuggled),
            "the ask never carries {smuggled:?} to the screen: {prompt:?}"
        );
    }
    assert!(
        prompt.contains('\u{FFFD}'),
        "what was replaced is shown as replaced, not dropped: {prompt:?}"
    );
    assert!(
        prompt.contains("step one\u{FFFD}step two"),
        "nothing is cut: the text around the replacement stays: {prompt:?}"
    );
    assert!(
        prompt.contains("hidden\nstep three"),
        "a newline stays a newline, so the plan's shape is what is approved: {prompt:?}"
    );
}

/// No token pre-answers a plan: the gate's ask offers no `[s]` grant, so the
/// prompt states no session line and no scope sentence for it.
#[test]
fn a_submit_plan_ask_offers_no_grant() {
    let facts = session_facts();
    let arguments = serde_json::json!({ "plan": "1. rewrite src/lib.rs" });
    assert_eq!(
        grant_token("submit_plan", &arguments, None, &facts),
        None,
        "a plan is not a call the grant store can pre-answer"
    );
}
