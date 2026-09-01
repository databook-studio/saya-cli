//! Bounded turn record assembly.
//!
//! Captures the facts of what occurred during a turn: user prompt, assistant answer,
//! observed objects mapped to turn-scoped identifiers (`T0..Tn`), user corrections,
//! override findings, and supplied contracts.

use saya_agent::OverrideFindingDto;
use serde::{Deserialize, Serialize};

use super::turn_table::TurnObjectTable;
use crate::agent::tools::DrainedObservations;
use crate::connection::ConnectionRegistry;
use crate::contracts::{RecallReceipt, SuppliedContract};

/// Maximum byte budgets for turn record text fields (Safety Property 3).
#[allow(dead_code)]
pub const MAX_PROMPT_BYTES: usize = 4096;
#[allow(dead_code)]
pub const MAX_ANSWER_BYTES: usize = 8192;
#[allow(dead_code)]
pub const MAX_TURN_RECORD_BYTES: usize = 16_384;

/// DTO summarizing a supplied claim from recall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SuppliedClaimDto {
    pub claim_id: String,
    pub kind: String,
    pub value: String,
    pub column: Option<String>,
    pub status: String,
}

/// DTO summarizing a supplied contract from recall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SuppliedContractDto {
    pub profile: String,
    pub object: String,
    pub schema_state: String,
    pub claims: Vec<SuppliedClaimDto>,
}

impl From<&SuppliedContract> for SuppliedContractDto {
    fn from(c: &SuppliedContract) -> Self {
        Self {
            profile: c.profile.clone(),
            object: c.object.clone(),
            schema_state: c.schema_state.to_string(),
            claims: c
                .claims
                .iter()
                .map(|cl| SuppliedClaimDto {
                    claim_id: cl.claim_id.as_str().to_string(),
                    kind: cl.kind.to_string(),
                    value: cl.value.clone(),
                    column: cl.column.clone(),
                    status: cl.status.as_str().to_string(),
                })
                .collect(),
        }
    }
}

/// A bounded record of turn execution facts for structured extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TurnRecord {
    pub prompt: String,
    pub assistant_answer: String,
    pub object_table: TurnObjectTable,
    pub user_corrections: Vec<String>,
    pub override_findings: Vec<OverrideFindingDto>,
    pub supplied_claims: Vec<SuppliedContractDto>,
}

impl TurnRecord {
    /// Assembles a `TurnRecord` from the turn's inputs, observations, and outputs.
    ///
    /// `registry` is the live connection set: an observation carries the opaque
    /// `ProfileIdentity` of the connection it ran against, but the object table
    /// is keyed by connection *name* (the resolver resolves by name), so the
    /// identity is turned into a name here. A connection whose identity no longer
    /// resolves — a dropped connection mid-turn — falls back to the primary so
    /// the object is not silently lost.
    #[allow(dead_code)]
    pub fn assemble(
        prompt: &str,
        assistant_answer: &str,
        registry: &ConnectionRegistry,
        observations: &DrainedObservations,
        receipt: Option<&RecallReceipt>,
        overrides: &[OverrideFindingDto],
    ) -> Self {
        let bounded_prompt = truncate_utf8(prompt, MAX_PROMPT_BYTES);
        let bounded_answer = truncate_utf8(assistant_answer, MAX_ANSWER_BYTES);

        let primary_profile = registry.primary_name();
        let mut object_table = TurnObjectTable::new();

        for obs in &observations.observations {
            // The observation records the identity; the table needs the name the
            // resolver keys on. `unwrap_or(primary_profile)` covers an
            // observation with no profile (a test connector) or one whose
            // connection has since been dropped.
            let prof = obs
                .profile
                .as_ref()
                .and_then(|p| registry.name_for_identity(p.as_str()))
                .unwrap_or(primary_profile);
            for obj_parts in &obs.objects {
                let qualified = obj_parts.join(".");
                object_table.register(prof, &qualified, &obs.columns);
            }
        }

        let mut supplied_dtos = Vec::new();
        if let Some(r) = receipt {
            for contract in &r.supplied {
                supplied_dtos.push(SuppliedContractDto::from(contract));
                let cols: Vec<String> = contract
                    .claims
                    .iter()
                    .filter_map(|c| c.column.clone())
                    .collect();
                object_table.register(&contract.profile, &contract.object, &cols);
            }
        }

        let user_corrections = extract_user_corrections(&bounded_prompt);

        Self {
            prompt: bounded_prompt,
            assistant_answer: bounded_answer,
            object_table,
            user_corrections,
            override_findings: overrides.to_vec(),
            supplied_claims: supplied_dtos,
        }
    }
}

/// Truncates a string to at most `max_bytes` at a valid UTF-8 character boundary.
#[allow(dead_code)]
fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    s[..boundary].to_string()
}

/// Extracts potential corrective sentences from user prompt.
#[allow(dead_code)]
fn extract_user_corrections(prompt: &str) -> Vec<String> {
    prompt
        .lines()
        .flat_map(|line| line.split(['.', '!', '?']))
        .map(str::trim)
        .filter(|s| {
            let lower = s.to_lowercase();
            lower.starts_with("no")
                || lower.starts_with("actually")
                || lower.starts_with("instead")
                || lower.starts_with("remember")
                || lower.contains("should be")
        })
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::ClaimId;

    #[test]
    fn test_turn_record_bounds_input_strings() {
        let huge_prompt = "p".repeat(50_000);
        let huge_answer = "a".repeat(50_000);
        let obs = DrainedObservations {
            observations: Vec::new(),
            truncated: false,
        };
        let registry = ConnectionRegistry::new("primary");

        let record = TurnRecord::assemble(&huge_prompt, &huge_answer, &registry, &obs, None, &[]);
        assert_eq!(record.prompt.len(), MAX_PROMPT_BYTES);
        assert_eq!(record.assistant_answer.len(), MAX_ANSWER_BYTES);
    }

    #[test]
    fn test_extract_user_corrections() {
        let prompt = "Actually, orders are sorted by date. No, use return_date.";
        let corrections = extract_user_corrections(prompt);
        assert_eq!(corrections.len(), 2);
    }

    #[test]
    fn test_dto_conversion_from_supplied_contract() {
        let contract = SuppliedContract {
            profile: "p1".into(),
            object: "catalog.public.orders".into(),
            schema_state: "current",
            claims: vec![crate::contracts::SuppliedClaim {
                claim_id: ClaimId::parse("c-1").unwrap(),
                kind: "default_time_column",
                value: "created_at".into(),
                column: Some("created_at".into()),
                status: saya_types::ClaimStatus::Confirmed,
            }],
        };

        let dto = SuppliedContractDto::from(&contract);
        assert_eq!(dto.profile, "p1");
        assert_eq!(dto.claims.len(), 1);
        assert_eq!(dto.claims[0].claim_id, "c-1");
        assert_eq!(dto.claims[0].value, "created_at");
    }
}
