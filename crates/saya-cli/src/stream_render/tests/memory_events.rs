/// KnowledgeSupplied maps to a real TerminalEvent variant (not
/// NotImplemented) and renders through the text adapter.
#[test]
fn knowledge_supplied_renders_through_the_text_adapter() {
    let event = AgentEvent::knowledge_supplied(
        KnowledgeOutcome::Ran {
            store_unavailable: false,
        },
        vec![dto_contract(vec![
            dto_claim("c-1", "table_alias", "orders", ClaimStatus::Confirmed),
            dto_claim(
                "c-2",
                "default_time_column",
                "created_at",
                ClaimStatus::Candidate,
            ),
        ])],
        0,
    );
    let rendered = render_agent(event, RenderFormat::Text, &mut false);
    // The compact header, the unconfirmed count (pointing at /queue),
    // and the per-claim lines all reach stdout through the adapter.
    assert!(
        rendered
            .stdout
            .contains("memory supplied · 2 claims (1 unconfirmed — review with /queue)"),
        "{:?}",
        rendered.stdout
    );
    assert!(
        rendered.stdout.contains("table_alias  orders  confirmed"),
        "{:?}",
        rendered.stdout
    );
    assert!(
        rendered.stdout.contains("candidate  (unconfirmed)"),
        "{:?}",
        rendered.stdout
    );
    assert_eq!(rendered.stderr, "");
}

/// The three outcomes render distinguishably through the text adapter, and
/// Ran-and-found-nothing is silent (spec §5 / §4).
#[test]
fn text_adapter_distinguishes_the_three_outcomes() {
    let off = render_agent(
        AgentEvent::knowledge_supplied(KnowledgeOutcome::Off, Vec::new(), 0),
        RenderFormat::Text,
        &mut false,
    );
    let skipped = render_agent(
        AgentEvent::knowledge_supplied(KnowledgeOutcome::Skipped, Vec::new(), 0),
        RenderFormat::Text,
        &mut false,
    );
    let ran_empty = render_agent(
        AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            Vec::new(),
            0,
        ),
        RenderFormat::Text,
        &mut false,
    );
    assert_eq!(off.stdout, "memory off · recall disabled\n");
    assert_eq!(
        skipped.stdout,
        "memory skipped · not permitted to read saved claims\n"
    );
    assert_eq!(ran_empty.stdout, "", "Ran-and-found-nothing is silent");
    assert_ne!(off.stdout, skipped.stdout);
}

/// A non-zero dropped count is visible through the text adapter (spec §5).
#[test]
fn text_adapter_shows_a_nonzero_dropped_count() {
    let rendered = render_agent(
        AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            vec![dto_contract(vec![dto_claim(
                "c-1",
                "table_alias",
                "orders",
                ClaimStatus::Confirmed,
            )])],
            30,
        ),
        RenderFormat::Text,
        &mut false,
    );
    assert!(
        rendered.stdout.contains("· 30 more dropped by bounds"),
        "{:?}",
        rendered.stdout
    );
}

/// The JSON/NDJSON adapter carries the event under its type tag rather than
/// the NotImplemented fallback (spec §2: every adapter that renders events).
#[test]
fn json_adapter_carries_the_event_under_its_type_tag() {
    let event = AgentEvent::knowledge_supplied(
        KnowledgeOutcome::Ran {
            store_unavailable: false,
        },
        vec![dto_contract(vec![dto_claim(
            "c-1",
            "table_alias",
            "orders",
            ClaimStatus::Confirmed,
        )])],
        0,
    );
    let te = terminal_event(event).expect("this event renders headlessly");
    let rendered = render_event(&te, RenderFormat::Json);
    assert!(
        rendered.stdout.contains(r#""event":"knowledge_supplied""#),
        "{:?}",
        rendered.stdout
    );
    // The NotImplemented fallback is not what the JSON adapter emits here.
    assert!(
        !rendered.stdout.contains("not_implemented"),
        "{:?}",
        rendered.stdout
    );
}
