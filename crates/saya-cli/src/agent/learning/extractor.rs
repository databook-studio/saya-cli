//! Structured extraction response parser and candidate validation.
//!
//! Parses LLM extraction JSON outputs, rejects hallucinated object IDs (Safety Property 2),
//! enforces maximum proposal limits (Safety Property 3), and sanitizes credentials.

use saya_types::KnowledgeSlot;

use super::extractor_schema::{
    ExtractedProposal, ExtractionError, ExtractionResponseJson, MAX_PROPOSALS_PER_EXTRACTION,
    ProposalOrigin, RawProposalJson, build_claim_payload,
};
use super::turn_table::{TurnObjectId, TurnObjectTable};

/// Parses a model extraction output string into typed `ExtractedProposal` records.
#[allow(dead_code)]
pub fn parse_extraction_response(
    raw: &str,
    table: &TurnObjectTable,
) -> Result<Vec<ExtractedProposal>, ExtractionError> {
    let unescaped = strip_markdown_fences(raw);
    let parsed: ExtractionResponseJson = serde_json::from_str(unescaped)
        .map_err(|e| ExtractionError::JsonParse(format!("{e}: {raw}")))?;

    let mut proposals = Vec::new();

    for raw_prop in parsed.proposals {
        if proposals.len() >= MAX_PROPOSALS_PER_EXTRACTION {
            break;
        }

        if let Some(prop) = convert_raw_proposal(raw_prop, table) {
            proposals.push(prop);
        }
    }

    Ok(proposals)
}

/// Converts a single raw JSON proposal into a validated `ExtractedProposal`,
/// discarding any proposal with invalid slots, hallucinated object IDs, or bad payloads.
#[allow(dead_code)]
fn convert_raw_proposal(
    raw: RawProposalJson,
    table: &TurnObjectTable,
) -> Option<ExtractedProposal> {
    let object_id = TurnObjectId::parse(&raw.object_id)?;
    table.get_by_id(&object_id)?;

    let slot = KnowledgeSlot::parse(&raw.slot)?;
    let value = build_claim_payload(&slot, &raw.value, raw.reason.as_deref()).ok()?;

    let origin = ProposalOrigin::parse(&raw.origin).unwrap_or(ProposalOrigin::AssistantInferred);
    let confidence = raw.confidence.unwrap_or(0.8).clamp(0.0, 1.0);

    Some(ExtractedProposal {
        object_id,
        slot,
        value,
        origin,
        confidence,
    })
}

/// Strips markdown fences (e.g. ````json... ````) from the LLM output.
#[allow(dead_code)]
fn strip_markdown_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```json")
        && let Some(inner) = rest.strip_suffix("```")
    {
        return inner.trim();
    }
    if let Some(rest) = trimmed.strip_prefix("```")
        && let Some(inner) = rest.strip_suffix("```")
    {
        return inner.trim();
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::{ClaimPayload, ColumnRole};

    fn setup_test_table() -> TurnObjectTable {
        let mut table = TurnObjectTable::new();
        table.register(
            "primary",
            "catalog.public.orders",
            &["id".into(), "created_at".into(), "shipped_at".into()],
        );
        table.register(
            "primary",
            "catalog.public.users",
            &["user_id".into(), "email".into()],
        );
        table
    }

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

    #[test]
    fn test_parse_handles_markdown_fenced_json() {
        let table = setup_test_table();
        let fenced = r#"```json
{
    "proposals": [
        {
            "object_id": "T0",
            "slot": "table.grain",
            "value": "one row per order",
            "origin": "user_explicit"
        }
    ]
}
```"#;

        let res = parse_extraction_response(fenced, &table).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].object_id, TurnObjectId::new(0));
    }

    #[test]
    fn test_parse_handles_malformed_json_gracefully() {
        let table = setup_test_table();
        let bad_json = r#"{"proposals": [ not valid json }"#;

        let res = parse_extraction_response(bad_json, &table);
        assert!(res.is_err());
        match res.unwrap_err() {
            ExtractionError::JsonParse(_) => (),
            other => panic!("Expected JsonParse error, got {other:?}"),
        }
    }

    /// Q3 / deliverable 5: JSON mode returns bare JSON (no fence); the default
    /// path returns ```` ```json ````-wrapped output. Not every provider honours
    /// the JSON hint, so the stripper stays and both shapes must parse.
    #[test]
    fn test_parse_handles_bare_json() {
        let table = setup_test_table();
        let bare = r#"{"proposals": [
            {
                "object_id": "T0",
                "slot": "table.grain",
                "value": "one row per order",
                "origin": "user_explicit"
            }
        ]}"#;

        let res = parse_extraction_response(bare, &table).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].object_id, TurnObjectId::new(0));
    }

    /// The stripper itself handles bare JSON, the ```json fence, and a bare ```
    /// fence — the three shapes a provider that ignores the JSON hint can still
    /// return. Both fences collapse to the same inner JSON; bare passes through.
    #[test]
    fn strip_markdown_fences_handles_bare_and_both_fence_flavors() {
        let bare = r#"  {"proposals": []}  "#;
        assert_eq!(strip_markdown_fences(bare), r#"{"proposals": []}"#);

        let json_fence = "```json\n{\"proposals\": []}\n```";
        assert_eq!(strip_markdown_fences(json_fence), r#"{"proposals": []}"#);

        let plain_fence = "```\n{\"proposals\": []}\n```";
        assert_eq!(strip_markdown_fences(plain_fence), r#"{"proposals": []}"#);
    }

    #[test]
    fn test_parse_rejects_secret_or_credential_values() {
        let table = setup_test_table();
        let json = r#"{
            "proposals": [
                {
                    "object_id": "T0",
                    "slot": "table.description",
                    "value": "Bearer sk-1234567890abcdef",
                    "origin": "assistant_inferred"
                },
                {
                    "object_id": "T0",
                    "slot": "table.description",
                    "value": "Valid table description",
                    "origin": "assistant_inferred"
                }
            ]
        }"#;

        let res = parse_extraction_response(json, &table).unwrap();
        assert_eq!(res.len(), 1, "Credential proposal must be rejected");
        assert_eq!(
            res[0].value,
            ClaimPayload::table_description("Valid table description").unwrap()
        );
    }
}
