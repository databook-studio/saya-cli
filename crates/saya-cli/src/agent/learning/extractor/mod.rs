//! Structured extraction response parser and candidate validation.
//!
//! Parses LLM extraction JSON outputs, rejects hallucinated object IDs (Safety Property 2),
//! enforces maximum proposal limits (Safety Property 3), and sanitizes credentials.

use super::extractor_schema::{
    ExtractedProposal, ExtractionError, ExtractionResponseJson, MAX_PROPOSALS_PER_EXTRACTION,
};
use super::turn_table::TurnObjectTable;

mod fences;
mod json;

/// Keep provider parse diagnostics structural: the response body may contain
/// rows, prompts, or credentials, so it must never be copied into an error.
pub(crate) const MAX_JSON_DIAGNOSTIC_BYTES: usize = 128;

/// Parses a model extraction output string into typed `ExtractedProposal` records.
#[allow(dead_code)]
pub fn parse_extraction_response(
    raw: &str,
    table: &TurnObjectTable,
) -> Result<Vec<ExtractedProposal>, ExtractionError> {
    let unescaped = fences::strip_markdown_fences(raw);
    let parsed: ExtractionResponseJson = serde_json::from_str(unescaped)
        .map_err(|e| ExtractionError::JsonParse(json::json_parse_diagnostic(&e)))?;

    let mut proposals = Vec::new();

    for raw_prop in parsed.proposals {
        if proposals.len() >= MAX_PROPOSALS_PER_EXTRACTION {
            break;
        }

        if let Some(prop) = json::convert_raw_proposal(raw_prop, table) {
            proposals.push(prop);
        }
    }

    Ok(proposals)
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
