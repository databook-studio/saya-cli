use super::super::extractor_schema::{
    ExtractedProposal, ProposalOrigin, RawProposalJson, build_claim_payload,
};
use super::super::turn_table::{TurnObjectId, TurnObjectTable};
use super::MAX_JSON_DIAGNOSTIC_BYTES;
use saya_types::KnowledgeSlot;

pub(super) fn json_parse_diagnostic(error: &serde_json::Error) -> String {
    let category = match error.classify() {
        serde_json::error::Category::Io => "io",
        serde_json::error::Category::Syntax => "syntax",
        serde_json::error::Category::Data => "data",
        serde_json::error::Category::Eof => "eof",
    };
    let diagnostic = format!(
        "invalid extraction JSON ({category}) at line {}, column {}",
        error.line(),
        error.column()
    );
    debug_assert!(diagnostic.len() <= MAX_JSON_DIAGNOSTIC_BYTES);
    diagnostic
}

/// Converts a single raw JSON proposal into a validated `ExtractedProposal`,
/// discarding any proposal with invalid slots, hallucinated object IDs, or bad payloads.
#[allow(dead_code)]
pub(super) fn convert_raw_proposal(
    raw: RawProposalJson,
    table: &TurnObjectTable,
) -> Option<ExtractedProposal> {
    let object_id = TurnObjectId::parse(&raw.object_id)?;
    table.get_by_id(&object_id)?;

    let slot = KnowledgeSlot::parse(&raw.slot)?;
    let value = build_claim_payload(&slot, &raw).ok()?;

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
