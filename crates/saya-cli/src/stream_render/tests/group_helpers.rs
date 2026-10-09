fn drain_text_stream(events: Vec<AgentEvent>) -> String {
    let mut open = false;
    let mut group = TextGroupState::new();
    let mut stdout = String::new();
    for event in events {
        let rendered = render_text_stream(event, RenderFormat::Text, &mut open, &mut group);
        stdout.push_str(&rendered.stdout);
    }
    stdout.push_str(&flush_group(&mut group, &mut open).stdout);
    stdout
}

fn drain_ndjson_stream(events: Vec<AgentEvent>) -> String {
    let mut open = false;
    let mut group = TextGroupState::new();
    let mut stdout = String::new();
    for event in events {
        let rendered = render_text_stream(event, RenderFormat::Ndjson, &mut open, &mut group);
        stdout.push_str(&rendered.stdout);
    }
    stdout.push_str(&flush_group(&mut group, &mut open).stdout);
    stdout
}

fn write_effect() -> ToolEffect {
    ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::WriteWorkspace,
    }
}

fn run_effect() -> ToolEffect {
    ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: false,
        local_state: LocalStateEffect::WriteWorkspace,
    }
}

fn mixed_sequence() -> Vec<AgentEvent> {
    vec![
        AgentEvent::assistant_text("the plan"),
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
        AgentEvent::assistant_text("done"),
        AgentEvent::complete(),
    ]
}
