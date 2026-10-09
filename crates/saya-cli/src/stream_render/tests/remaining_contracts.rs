/// Property 2 (piped half): `approvals_are_untouched` — a ToolDenied
/// boundary flushes the open group first and renders the denial line
/// byte-identical to the ungrouped path, so a call can never collapse
/// into a group that swallows a consent moment. The prompt/answer half
/// lives beside the seam that owns it
/// (`prompt_approval_tests::ask_prompt_keeps_its_bytes_and_answers_under_grouping`);
/// the bypass-line half beside its seam
/// (`interactive::session::permissions::activation::tests::grouping::bypass_line_keeps_its_bytes_under_grouping`):
/// the grouper never sees either string, so grouping cannot touch them.
#[test]
fn approvals_are_untouched() {
    let events = vec![
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "a.md", "content": "hi"}),
            Some(write_effect()),
        ),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "a.md written".into(),
        },
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "b.md", "content": "hi"}),
            Some(write_effect()),
        ),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "b.md written".into(),
        },
        AgentEvent::ToolDenied {
            name: "run_command".into(),
            reason: "denied".into(),
        },
        AgentEvent::complete(),
    ];
    let stdout = drain_text_stream(events);
    assert!(
        stdout.contains("2 tool calls · ok"),
        "the pre-denial run collapses on its own: {stdout:?}"
    );
    let denied = AgentEvent::ToolDenied {
        name: "run_command".into(),
        reason: "denied".into(),
    };
    let mut open = false;
    let ungrouped = render_agent(denied, RenderFormat::Text, &mut open).stdout;
    assert_eq!(
        ungrouped, "Approval denied for run_command: denied\n",
        "precondition: the denial line under test"
    );
    assert!(
        stdout.contains(&ungrouped),
        "the denial renders byte-identical after the flush: {stdout:?}"
    );
    assert!(
        stdout.find("2 tool calls · ok").expect("summary")
            < stdout.find("Approval denied").expect("denial"),
        "the summary flushes before the boundary: {stdout:?}"
    );
    assert!(
        !stdout.contains("▸ 3 tool calls"),
        "the denied call never joins the collapsed count: {stdout:?}"
    );
}

/// The clarification event (B3c) reaches the NDJSON/JSON stream under its
/// own type tag with the question and options carried, and the text
/// adapter prints the question with the options numbered — never the
/// `unrecognized agent event` catch-all.
#[test]
fn clarification_needed_reaches_ndjson_and_prints_in_text() {
    let event = AgentEvent::clarification_needed(
        "Which metric should \"active users\" use?",
        vec![
            "sessions in the last 30 days".into(),
            "purchases in the last 90 days".into(),
        ],
    );
    let terminal = terminal_event(event).expect("the clarification renders headlessly");
    assert!(
        !matches!(terminal, TerminalEvent::NotImplemented { .. }),
        "ClarificationNeeded must not fall through to the catch-all: {terminal:?}"
    );
    let json = render_event(&terminal, RenderFormat::Ndjson);
    assert!(
        json.stdout.contains(r#""event":"clarification_needed""#),
        "ndjson must tag the clarification: {json:?}"
    );
    assert!(
        json.stdout
            .contains("Which metric should \\\"active users\\\" use?")
            && json.stdout.contains("sessions in the last 30 days"),
        "the question and the options are carried: {json:?}"
    );
    let text = render_event(&terminal, RenderFormat::Text);
    assert!(
        text.stdout
            .contains("Which metric should \"active users\" use?"),
        "the question prints in text: {:?}",
        text.stdout
    );
    assert!(
        text.stdout.contains("1. sessions in the last 30 days")
            && text.stdout.contains("2. purchases in the last 90 days"),
        "the options print numbered: {:?}",
        text.stdout
    );
    assert_eq!(text.stderr, "", "nothing on stderr");
}

/// A clarification event is a boundary, not a group member: the piped text
/// surface flushes any buffered tool run before printing the question, so
/// the ask's own request/completion lines land above it and the question
/// stands alone.
#[test]
fn the_piped_text_surface_flushes_the_ask_group_before_the_question() {
    let events = vec![
        AgentEvent::tool_requested(
            "request_clarification",
            serde_json::json!({"question": "which metric?"}),
            Some(ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            }),
        ),
        AgentEvent::ToolCompleted {
            name: "request_clarification".into(),
            summary: "question asked — the turn pauses for the user's answer".into(),
        },
        AgentEvent::clarification_needed("which metric?", Vec::new()),
    ];
    let stdout = drain_text_stream(events);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("request_clarification")),
        "the ask's group flushed before the question: {stdout:?}"
    );
    assert!(
        stdout.contains("which metric?"),
        "the question prints after the group: {stdout:?}"
    );
    assert!(
        stdout.rfind("which metric?").expect("the question")
            > stdout.find("request_clarification").expect("the ask line"),
        "the question follows the flushed group: {stdout:?}"
    );
}
