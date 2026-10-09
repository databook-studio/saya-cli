// --- Spec A1: the KnowledgeOverridden shaper. ---

fn override_finding(id: &str, claimed: &str, observed: &[&str]) -> OverrideFindingDto {
    OverrideFindingDto {
        claim_id: ClaimId::parse(id).unwrap(),
        kind: "default_time_column".into(),
        claimed_value: claimed.into(),
        observed_columns: observed.iter().map(|s| s.to_string()).collect(),
    }
}

/// Empty findings render nothing.
#[test]
fn no_findings_render_nothing() {
    assert_eq!(knowledge_overridden_text(&[]), "");
}

/// One finding renders the header and one line that names the referenced
/// column and what the claim specified.
#[test]
fn one_finding_names_the_referenced_column_and_the_specified_value() {
    let text = knowledge_overridden_text(&[override_finding(
        "c-rental-time",
        "return_date",
        &["rental_date"],
    )]);
    assert!(
        text.contains("memory overridden · 1 finding"),
        "header: {text}"
    );
    assert!(
        text.contains("referenced rental_date"),
        "names the column the SQL referenced: {text}"
    );
    assert!(
        text.contains("where you specified return_date"),
        "names what the claim specified: {text}"
    );
    assert!(
        text.contains("default_time_column"),
        "names the kind: {text}"
    );
}

/// Multiple findings pluralize the header and render one line each.
#[test]
fn multiple_findings_pluralize_the_header() {
    let text = knowledge_overridden_text(&[
        override_finding("c-a", "return_date", &["rental_date"]),
        override_finding("c-b", "created_at", &["updated_at"]),
    ]);
    assert!(
        text.contains("memory overridden · 2 findings"),
        "pluralized header: {text}"
    );
    assert!(text.contains("referenced rental_date"), "{text}");
    assert!(text.contains("referenced updated_at"), "{text}");
}

// Test 5: the rendered text does not contain "used" as a causal assertion
// about the time column. The extractor cannot tell a predicate from a
// projection, so the line says "referenced", never "used". This is the
// wording constraint the spec checks hardest.
#[test]
fn the_rendered_text_does_not_assert_the_model_used_a_time_column() {
    let text = knowledge_overridden_text(&[override_finding(
        "c-rental-time",
        "return_date",
        &["rental_date"],
    )]);
    // The line says the SQL *referenced* a column, never that SAYA *used*
    // one as the time column — the role is unknowable from names.
    assert!(
        !text.contains("used"),
        "the shaper must not assert a causal 'used' about the time column: {text}"
    );
    assert!(
        text.contains("referenced"),
        "the shaper says 'referenced': {text}"
    );
}

/// The event serializes under its `knowledge_overridden` type tag and never
/// carries an opaque identity (the DTO has no such field, by construction).
#[test]
fn overridden_event_serializes_with_type_tag_and_no_identity() {
    let event = AgentEvent::knowledge_overridden(vec![override_finding(
        "c-rental-time",
        "return_date",
        &["rental_date"],
    )]);
    let json = serde_json::to_string(&event).expect("serializes");
    assert!(
        json.contains(r#""type":"knowledge_overridden""#),
        "type tag: {json}"
    );
    let fake_identity = "sha256:9f2a8c7b1e4d0a6f3c5b8e2d7a9f1c4b6e8a0d2f4c6b8e0a2d4f6c8b0e2d4f6";
    assert!(
        !json.contains(fake_identity),
        "opaque identity leaked into the event: {json}"
    );
}
