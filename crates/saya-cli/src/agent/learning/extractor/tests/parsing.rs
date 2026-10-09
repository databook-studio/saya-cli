use super::*;

#[test]
fn test_parse_valid_json_proposals() {
    let table = setup_test_table();
    let json = r#"{
            "proposals": [
                {
                    "object_id": "T0",
                    "slot": "table.grain",
                    "value": "one row per completed customer order",
                    "origin": "user_explicit",
                    "confidence": 1.0
                },
                {
                    "object_id": "T0",
                    "slot": "table.default_time",
                    "value": "created_at",
                    "origin": "assistant_inferred",
                    "confidence": 0.95
                },
                {
                    "object_id": "T1",
                    "slot": "column:user_id.role",
                    "value": "identifier",
                    "origin": "assistant_inferred",
                    "confidence": 0.9
                }
            ]
        }"#;

    let res = parse_extraction_response(json, &table).unwrap();
    assert_eq!(res.len(), 3);

    assert_eq!(res[0].object_id, TurnObjectId::new(0));
    assert_eq!(res[0].slot, KnowledgeSlot::TableGrain);
    assert_eq!(
        res[0].value,
        ClaimPayload::table_grain("one row per completed customer order", None).unwrap()
    );
    assert_eq!(res[0].origin, ProposalOrigin::UserExplicit);
    assert_eq!(res[0].confidence, 1.0);

    assert_eq!(res[1].object_id, TurnObjectId::new(0));
    assert_eq!(res[1].slot, KnowledgeSlot::TableDefaultTime);
    assert_eq!(
        res[1].value,
        ClaimPayload::default_time_column("created_at", None).unwrap()
    );

    assert_eq!(res[2].object_id, TurnObjectId::new(1));
    assert_eq!(
        res[2].slot,
        KnowledgeSlot::ColumnRole {
            column: "user_id".into()
        }
    );
    assert_eq!(
        res[2].value,
        ClaimPayload::column_role("user_id", ColumnRole::Identifier, None).unwrap()
    );
}

#[test]
fn test_parse_carries_reason_onto_a_directive_payload() {
    // A user who states "use return_date — a rental only counts once it
    // comes back" states one fact with one reason; both halves must land on
    // one claim. The model emits the reason as a `reason` field; the parser
    // forwards it to the directive constructor.
    let table = setup_test_table();
    let json = r#"{
            "proposals": [
                {
                    "object_id": "T0",
                    "slot": "table.default_time",
                    "value": "created_at",
                    "reason": "a rental only counts once it comes back",
                    "origin": "user_explicit",
                    "confidence": 1.0
                }
            ]
        }"#;
    let res = parse_extraction_response(json, &table).unwrap();
    assert_eq!(res.len(), 1);
    // `DefaultTimeColumn` is `#[non_exhaustive]`, so the match uses `..`;
    // `claim_value` is the single source of the rendered value, so reading
    // the column through it avoids moving the payload out of the vec.
    let (column, value) = crate::agent::recall_context::claim_value(&res[0].value);
    assert_eq!(column, None);
    assert_eq!(value, "created_at");
    // The reason is on the payload, not on the rendered value.
    assert!(matches!(
        &res[0].value,
        ClaimPayload::DefaultTimeColumn { reason, .. }
        if reason.as_deref() == Some("a rental only counts once it comes back")
    ));
}

#[test]
fn test_parse_drops_reason_for_a_non_directive_slot() {
    // A reason on a description is not applicable (the description
    // constructor takes none); the payload is still built, without a reason.
    let table = setup_test_table();
    let json = r#"{
            "proposals": [
                {
                    "object_id": "T0",
                    "slot": "table.description",
                    "value": "the orders table",
                    "reason": "ignored here",
                    "origin": "assistant_inferred"
                }
            ]
        }"#;
    let res = parse_extraction_response(json, &table).unwrap();
    assert_eq!(res.len(), 1);
    assert!(matches!(
        &res[0].value,
        ClaimPayload::TableDescription { text, .. } if text == "the orders table"
    ));
}

#[test]
fn test_parse_rejects_hallucinated_object_id() {
    let table = setup_test_table();
    let json = r#"{
            "proposals": [
                {
                    "object_id": "T99",
                    "slot": "table.grain",
                    "value": "one row per non-existent entity",
                    "origin": "user_explicit"
                },
                {
                    "object_id": "T0",
                    "slot": "table.grain",
                    "value": "one row per real order",
                    "origin": "user_explicit"
                }
            ]
        }"#;

    let res = parse_extraction_response(json, &table).unwrap();
    assert_eq!(res.len(), 1, "Hallucinated T99 must be discarded");
    assert_eq!(res[0].object_id, TurnObjectId::new(0));
}

#[test]
fn test_parse_caps_proposals_at_eight() {
    let table = setup_test_table();
    let mut proposals_json = Vec::new();
    for i in 0..12 {
        proposals_json.push(format!(
                r#"{{"object_id": "T0", "slot": "column:created_at.description", "value": "Description {i}", "origin": "assistant_inferred"}}"#
            ));
    }
    let json = format!(r#"{{"proposals": [{}]}}"#, proposals_json.join(", "));

    let res = parse_extraction_response(&json, &table).unwrap();
    assert_eq!(res.len(), MAX_PROPOSALS_PER_EXTRACTION);
    assert_eq!(res.len(), 8);
}
