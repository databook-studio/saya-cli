/// KnowledgeOverridden maps to a real TerminalEvent variant (not
/// NotImplemented) and renders through the text adapter.
#[test]
fn knowledge_overridden_renders_through_the_text_adapter() {
    let event = AgentEvent::knowledge_overridden(vec![OverrideFindingDto {
        claim_id: ClaimId::parse("c-rental-time").unwrap(),
        kind: "default_time_column".into(),
        claimed_value: "return_date".into(),
        observed_columns: vec!["rental_date".into()],
    }]);
    let rendered = render_agent(event, RenderFormat::Text, &mut false);
    assert!(
        rendered.stdout.contains("memory overridden · 1 finding"),
        "{:?}",
        rendered.stdout
    );
    assert!(
        rendered.stdout.contains("referenced rental_date"),
        "{:?}",
        rendered.stdout
    );
    // The wording constraint: no causal "used" about the time column.
    assert!(
        !rendered.stdout.contains("used"),
        "the text adapter must not assert a causal 'used': {:?}",
        rendered.stdout
    );
    assert_eq!(rendered.stderr, "");
}

/// The JSON/NDJSON adapter carries KnowledgeOverridden under its type tag
/// rather than the NotImplemented fallback (spec §2: every adapter).
#[test]
fn json_adapter_carries_the_overridden_event_under_its_type_tag() {
    let event = AgentEvent::knowledge_overridden(vec![OverrideFindingDto {
        claim_id: ClaimId::parse("c-rental-time").unwrap(),
        kind: "default_time_column".into(),
        claimed_value: "return_date".into(),
        observed_columns: vec!["rental_date".into()],
    }]);
    let te = terminal_event(event).expect("this event renders headlessly");
    let rendered = render_event(&te, RenderFormat::Json);
    assert!(
        rendered
            .stdout
            .contains(r#""event":"knowledge_overridden""#),
        "{:?}",
        rendered.stdout
    );
    assert!(
        !rendered.stdout.contains("not_implemented"),
        "{:?}",
        rendered.stdout
    );
}

/// KnowledgeLearningSkipped maps to a real TerminalEvent variant (not
/// NotImplemented) and renders the spec line through the text adapter
/// (packet-54 decision 4 — both adapters render it).
#[test]
fn knowledge_learning_skipped_renders_through_the_text_adapter() {
    let event = AgentEvent::knowledge_learning_skipped(LearningSkipReason::TimedOut);
    let rendered = render_agent(event, RenderFormat::Text, &mut false);
    assert!(
        rendered
            .stdout
            .contains("memory not recorded · extraction timed out"),
        "text adapter renders the timeout line: {:?}",
        rendered.stdout
    );
    assert_eq!(rendered.stderr, "");

    let event = AgentEvent::knowledge_learning_skipped(LearningSkipReason::Failed);
    let rendered = render_agent(event, RenderFormat::Text, &mut false);
    assert!(
        rendered
            .stdout
            .contains("memory not recorded · extraction failed"),
        "text adapter renders the failure line: {:?}",
        rendered.stdout
    );
}

/// The JSON/NDJSON adapter carries KnowledgeLearningSkipped under its type
/// tag rather than the NotImplemented fallback (packet-54 decision 4: every
/// adapter that renders events — the headless path previously would have
/// printed `unrecognized agent event`).
#[test]
fn json_adapter_carries_the_learning_skipped_event_under_its_type_tag() {
    let event = AgentEvent::knowledge_learning_skipped(LearningSkipReason::TimedOut);
    let te = terminal_event(event).expect("this event renders headlessly");
    let rendered = render_event(&te, RenderFormat::Json);
    assert!(
        rendered
            .stdout
            .contains(r#""event":"knowledge_learning_skipped""#),
        "type tag: {:?}",
        rendered.stdout
    );
    assert!(
        rendered.stdout.contains(r#""reason":"timed_out""#),
        "reason carried: {:?}",
        rendered.stdout
    );
    assert!(
        !rendered.stdout.contains("not_implemented"),
        "must not fall through to NotImplemented: {:?}",
        rendered.stdout
    );
}

/// KnowledgeLearningDisabled maps to a real TerminalEvent variant (not
/// NotImplemented) and renders the spec line through the text adapter.
#[test]
fn knowledge_learning_disabled_renders_through_the_text_adapter() {
    let event = AgentEvent::knowledge_learning_disabled("glm-5.2", 2);
    let rendered = render_agent(event, RenderFormat::Text, &mut false);
    assert!(
        rendered
            .stdout
            .contains("memory: learning disabled for this session"),
        "text adapter renders the disabled line: {:?}",
        rendered.stdout
    );
    assert!(
        rendered.stdout.contains("glm-5.2") && rendered.stdout.contains('2'),
        "the model and miss count appear: {:?}",
        rendered.stdout
    );
    assert_eq!(rendered.stderr, "");
}

/// The JSON/NDJSON adapter carries KnowledgeLearningDisabled under its
/// type tag rather than the NotImplemented fallback, with the model and
/// miss count on the wire.
#[test]
fn json_adapter_carries_the_learning_disabled_event_under_its_type_tag() {
    let event = AgentEvent::knowledge_learning_disabled("glm-5.2", 2);
    let te = terminal_event(event).expect("this event renders headlessly");
    let rendered = render_event(&te, RenderFormat::Json);
    assert!(
        rendered
            .stdout
            .contains(r#""event":"knowledge_learning_disabled""#),
        "type tag: {:?}",
        rendered.stdout
    );
    assert!(
        rendered.stdout.contains(r#""model":"glm-5.2""#),
        "model carried: {:?}",
        rendered.stdout
    );
    assert!(
        rendered.stdout.contains(r#""misses":2"#),
        "misses carried: {:?}",
        rendered.stdout
    );
    assert!(
        !rendered.stdout.contains("not_implemented"),
        "{:?}",
        rendered.stdout
    );
}
