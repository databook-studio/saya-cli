/// C2 property 1: a run of successful calls through the piped text
/// surface emits one summary line, not a line per call.
#[test]
fn piped_text_run_of_successful_calls_emits_one_summary_line() {
    let write = ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::WriteWorkspace,
    };
    let events = vec![
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": "hi"}),
            Some(write),
        ),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "notes.md written".into(),
        },
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "other.md", "content": "hi"}),
            Some(write),
        ),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "other.md written".into(),
        },
    ];
    let stdout = drain_text_stream(events);
    assert_eq!(
        stdout.lines().count(),
        1,
        "a run of successful calls is one summary line, not a line per call: {stdout:?}"
    );
    assert!(
        stdout.contains("2 tool calls · ok"),
        "the summary line collapses the run: {stdout:?}"
    );
}

/// C2 property 2: a group with a failure emits the header plus that
/// failure's full pair — today's request and completion lines verbatim.
#[test]
fn piped_text_group_with_failure_emits_header_plus_full_pair() {
    let events = vec![
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": "hi"}),
            Some(write_effect()),
        ),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "notes.md written".into(),
        },
        AgentEvent::tool_requested(
            "run_command",
            serde_json::json!({"program": "pytest", "args": ["-q"]}),
            Some(run_effect()),
        ),
        AgentEvent::ToolCompleted {
            name: "run_command".into(),
            summary: "failed pytest".into(),
        },
        AgentEvent::complete(),
    ];
    let stdout = drain_text_stream(events);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec![
            "▸ 2 tool calls · 1 failed (run_command [pytest -q]) — details below",
            "Using tool: run_command",
            "  [pytest -q]",
            "run_command: failed pytest",
        ],
        "header plus the failure's full pair, successes collapsed: {stdout:?}"
    );
}

/// C2 property 3: a one-member group emits today's lines, byte for byte.
#[test]
fn piped_text_single_call_matches_todays_lines_byte_for_byte() {
    let arguments = serde_json::json!({"path": "notes.md", "content": "hi"});
    let events = vec![
        AgentEvent::tool_requested("workspace_write", arguments, Some(write_effect())),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "notes.md written".into(),
        },
        AgentEvent::complete(),
    ];
    let grouped = drain_text_stream(events);
    let mut open = false;
    let ungrouped = format!(
        "{}{}",
        render_agent(
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "notes.md", "content": "hi"}),
                Some(write_effect()),
            ),
            RenderFormat::Text,
            &mut open,
        )
        .stdout,
        render_agent(
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "notes.md written".into(),
            },
            RenderFormat::Text,
            &mut open,
        )
        .stdout,
    );
    assert_eq!(
        grouped, ungrouped,
        "a one-member group must keep today's bytes: grouped={grouped:?}"
    );
}

/// C2 property 4: NDJSON output is byte-identical to before this slice —
/// the same event sequence renders event-for-event through the bypass,
/// with no grouping applied.
#[test]
fn ndjson_output_is_byte_identical_with_groups_failures_and_boundaries() {
    let events = mixed_sequence();
    let grouped = drain_ndjson_stream(events.clone());
    let mut open = false;
    let ungrouped: String = events
        .into_iter()
        .map(|event| render_agent(event, RenderFormat::Ndjson, &mut open).stdout)
        .collect();
    assert_eq!(
        grouped, ungrouped,
        "NDJSON must bypass the grouper entirely"
    );
    assert!(
        grouped.lines().count() >= 6,
        "the sequence must contain groups, failures and boundaries: {grouped:?}"
    );
    assert!(
        grouped.contains(r#""event":"tool_requested""#)
            && grouped.contains(r#""event":"tool_completed""#),
        "tool events stay one-per-line on the machine surface: {grouped:?}"
    );
    assert!(
        !grouped.contains("tool calls ·"),
        "no summary line may leak into NDJSON: {grouped:?}"
    );
}
