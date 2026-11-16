#[test]
fn text_status_closes_an_open_delta_line_once() {
    let mut open = false;
    assert_eq!(
        render_agent(
            AgentEvent::assistant_text("thinking"),
            RenderFormat::Text,
            &mut open
        )
        .stdout,
        "thinking"
    );
    // `schema_discovery`'s declared effect: touches nothing and reaches
    // nothing, so the read-only claim is earned and the line keeps it.
    assert_eq!(
        render_agent(
            AgentEvent::tool_requested(
                "schema",
                serde_json::Value::Null,
                Some(ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: LocalStateEffect::None,
                }),
            ),
            RenderFormat::Text,
            &mut open
        )
        .stdout,
        "\nUsing read-only tool: schema\n"
    );
    assert_eq!(
        render_agent(AgentEvent::complete(), RenderFormat::Text, &mut open).stdout,
        ""
    );
}

/// The write-shaped class — `write` declares
/// `LocalStateEffect::WriteWorkspace` — must never be announced as
/// read-only. The label is derived from the declaration the loop carries on
/// the event; a run that wrote a file must not announce "Using read-only
/// tool: write" at the exact moment it is about to write.
#[test]
fn a_write_shaped_tool_is_never_announced_as_read_only() {
    let rendered = render_agent(
        AgentEvent::tool_requested(
            "write",
            serde_json::json!({"path": "notes.md", "content": "the run's note"}),
            Some(ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::WriteWorkspace,
            }),
        ),
        RenderFormat::Text,
        &mut false,
    );
    assert_eq!(
        rendered.stdout, "Using tool: write\n  notes.md\n",
        "a write-shaped tool gets the claim-free line with the named file: {:?}",
        rendered.stdout
    );
    assert!(
        !rendered.stdout.contains("read-only"),
        "the read-only claim must not appear for a write-shaped tool: {:?}",
        rendered.stdout
    );
    assert_eq!(rendered.stderr, "");
}

/// A side-effecting tool — one whose declaration carries
/// `external_side_effect`, like `http_fetch` and `http_download` — must
/// never be announced as read-only either, with or without a visible call
/// detail (the detail arm names the program the call runs). The code
/// printed "Using read-only tool" for every tool, so both assertions failed
/// against it.
#[test]
fn a_side_effecting_tool_is_never_announced_as_read_only() {
    let effect = ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: false,
        local_state: LocalStateEffect::None,
    };
    let fetched = render_agent(
        AgentEvent::tool_requested(
            "http_fetch",
            serde_json::json!({"url": "https://example.com/feed"}),
            Some(effect),
        ),
        RenderFormat::Text,
        &mut false,
    );
    assert_eq!(
        fetched.stdout, "Using tool: http_fetch\n",
        "a side-effecting tool gets the claim-free line: {:?}",
        fetched.stdout
    );
    assert!(
        !fetched.stdout.contains("read-only"),
        "the read-only claim must not appear for a side-effecting tool: {:?}",
        fetched.stdout
    );

    // The detail arm: a side-effecting tool with a visible call detail
    // keeps the detail line and still carries no read-only claim.
    let detailed = render_agent(
        AgentEvent::tool_requested(
            "run_command",
            serde_json::json!({"program": "pytest", "args": ["-q"]}),
            Some(effect),
        ),
        RenderFormat::Text,
        &mut false,
    );
    assert_eq!(
        detailed.stdout, "Using tool: run_command\n  pytest\n",
        "the detail survives on the claim-free line: {:?}",
        detailed.stdout
    );
    assert!(
        !detailed.stdout.contains("read-only"),
        "the read-only claim must not appear beside a detail either: {:?}",
        detailed.stdout
    );
}
#[test]
fn ndjson_keeps_delta_and_complete_envelopes() {
    let mut open = false;
    assert_eq!(
        render_agent(
            AgentEvent::assistant_text("x"),
            RenderFormat::Ndjson,
            &mut open
        )
        .stdout,
        "{\"event\":\"assistant_text\",\"text\":\"x\"}\n"
    );
    assert_eq!(
        render_agent(AgentEvent::complete(), RenderFormat::Ndjson, &mut open).stdout,
        "{\"event\":\"complete\"}\n"
    );
}
#[test]
fn text_complete_closes_an_open_delta_line_once() {
    let mut open = false;
    let _ = render_agent(
        AgentEvent::assistant_text("done"),
        RenderFormat::Text,
        &mut open,
    );
    assert_eq!(
        render_agent(AgentEvent::complete(), RenderFormat::Text, &mut open).stdout,
        "\n"
    );
}
