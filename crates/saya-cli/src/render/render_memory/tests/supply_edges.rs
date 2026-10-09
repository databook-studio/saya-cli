/// `store_unavailable` renders distinctly and quietly — not silence, not an
/// error — and stays distinguishable from Ran-nothing and Skipped (Gap).
#[test]
fn store_unavailable_renders_distinctly() {
    let store_down = knowledge_supplied_text(ran(true), &[], 0);
    let ran_empty = knowledge_supplied_text(ran(false), &[], 0);
    let skipped = knowledge_supplied_text(KnowledgeOutcome::Skipped, &[], 0);
    assert!(
        store_down.contains("memory supplied · store unavailable"),
        "{store_down}"
    );
    assert_ne!(store_down, ran_empty);
    assert_ne!(store_down, skipped);
}

/// No opaque profile identity reaches output: the human-facing name appears,
/// a fabricated identity string does not (spec §5 / §4). The DTO has no
/// identity field by construction; this asserts the rendered text inherits
/// that guarantee.
#[test]
fn no_opaque_identity_reaches_output() {
    let fake_identity = "sha256:9f2a8c7b1e4d0a6f3c5b8e2d7a9f1c4b6e8a0d2f4c6b8e0a2d4f6c8b0e2d4f6";
    let text = knowledge_supplied_text(
        ran(false),
        &[contract(
            "analytics",
            "catalog.public.orders",
            "current",
            vec![claim(
                "c-1",
                "table_alias",
                "orders",
                None,
                ClaimStatus::Confirmed,
            )],
        )],
        0,
    );
    assert!(text.contains("analytics"), "profile name appears: {text}");
    assert!(
        !text.contains(fake_identity),
        "opaque identity leaked: {text}"
    );
}

/// The event serializes under its `knowledge_supplied` type tag (JSON/NDJSON
/// adapters fall out of the serde derive on `TerminalEvent`).
#[test]
fn event_serializes_with_type_tag() {
    let event = AgentEvent::knowledge_supplied(
        ran(false),
        vec![contract(
            "analytics",
            "catalog.public.orders",
            "current",
            vec![claim(
                "c-1",
                "table_alias",
                "orders",
                None,
                ClaimStatus::Candidate,
            )],
        )],
        0,
    );
    let json = serde_json::to_string(&event).expect("serializes");
    assert!(json.contains(r#""type":"knowledge_supplied""#), "{json}");
    assert!(json.contains("supplied"), "{json}");
}

/// Each claim line shows the short claim-id prefix: it is the
/// reference a user types into `/confirm`/`/reject`/`/use` to act on the
/// claim from this turn. The full 64-char id never appears — the prefix is
/// the reference, and `abbreviate_id` keeps it to the same width
/// `contracts list` shows.
#[test]
fn claim_line_shows_the_short_id_prefix_not_the_full_id() {
    // A realistic 67-char id (`c-` + 64 hex). `abbreviate_id` keeps the
    // first 6 chars + `…`.
    let long_id = "c-a86a3f0e9d7c5b4a2f0e9d7c5b4a2f0e9d7c5b4a2f0e9d7c5b4a2f0e9d7c5b4a";
    let text = knowledge_supplied_text(
        ran(false),
        &[contract(
            "analytics",
            "catalog.public.orders",
            "current",
            vec![claim(
                long_id,
                "table_alias",
                "orders",
                None,
                ClaimStatus::Candidate,
            )],
        )],
        0,
    );
    assert!(
        text.contains("c-a86a…"),
        "short prefix must appear so the user can type it: {text}"
    );
    assert!(
        !text.contains(&long_id[7..]),
        "the full id beyond the prefix must not appear: {text}"
    );
}
