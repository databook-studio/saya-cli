//! The `contract_propose` agent tool: the first tool that writes local state.
//!
//! Reuses `crate::contracts::args` for the qualified name, the kind-word
//! parser, and the kind→payload mapping (no second parser, no second
//! vocabulary). Enforces the spec 3c §2 policy: origin is always
//! `AssistantInferred`, status is always `Candidate`, at most eight proposals
//! per request scope, malformed/oversized input is refused before persistence,
//! and a duplicate returns the existing id and status. The opaque profile
//! identity never appears in a result — only the profile name does.
//!
//! `ToolError` lives in `saya-agent` (untouchable here) and has no variant for
//! a per-turn limit or a store/propose failure; the least-bad payload-free
//! mappings are `InvalidQueryArguments` for input-shape problems and
//! `QueryFailed` for runtime refusals — both payload-free, so an offending
//! value is never echoed (SPEC REVIEW, 3c).

mod definition;
mod evidence;
mod mapping;
mod validation;

pub(crate) use definition::definition as propose_definition;

use saya_agent::ToolError;
use saya_store::{ClaimEvidence, EvidenceKind, ProposeClaim};
use saya_types::{
    ClaimOrigin, ClaimStatus, DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity,
};

use super::DatabaseTools;
use crate::commands::unobserved_fingerprint;
use crate::contracts::args::{QualifiedName, build_payload, parse_kind, parse_qualified};
use crate::contracts::propose as propose_op;

use evidence::{TOUCHED, UNTOUCHED};
use mapping::{now_unix_ms, outcome_payload};
use validation::parse_arguments;

/// At most this many candidate proposals are stored per request scope; the
/// ninth is refused with a typed error (spec 3c §2).
const MAX_CANDIDATES_PER_TURN: usize = 8;

impl DatabaseTools {
    /// Executes `contract_propose`: validates, bounds the turn, resolves the
    /// connection, builds a `Candidate`/`AssistantInferred` claim, picks an
    /// evidence kind from the turn's observations, and persists it.
    pub(super) async fn execute_contract_propose(
        &self,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        // Fail closed: the tool is hidden when the privacy gate is closed, but a
        // model can still attempt it by name. A proposal is database-derived, so a
        // closed gate refuses it (a typed error) rather than write or return empty.
        if !self.allow_query_data {
            return Err(ToolError::DataSharingDisabled);
        }
        let args = parse_arguments(&arguments)?;
        let entry = self.registry.resolve(args.connection)?;
        let identity = entry
            .profile_id
            .as_deref()
            .ok_or(ToolError::NoConnectionSelected)?;
        let identity =
            ProfileIdentity::parse(identity).map_err(|_| ToolError::InvalidQueryArguments)?;
        let kind = parse_kind(args.kind).ok_or(ToolError::InvalidQueryArguments)?;
        let payload = build_payload(kind, args.value, args.column)
            .map_err(|_| ToolError::InvalidQueryArguments)?;
        let qualified =
            parse_qualified(args.table).map_err(|_| ToolError::InvalidQueryArguments)?;
        let object = DatabaseObjectRef::new(
            identity.clone(),
            &qualified.catalog,
            &qualified.schema,
            &qualified.object,
            DatabaseObjectKind::Table,
        )
        .map_err(|_| ToolError::InvalidQueryArguments)?;

        // Bound the turn before any persistence: the ninth attempt is refused,
        // not silently dropped. Counted per `DatabaseTools` (the request scope).
        if self
            .candidate_proposals
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            >= MAX_CANDIDATES_PER_TURN
        {
            return Err(ToolError::QueryFailed);
        }

        let request = ProposeClaim {
            object,
            fingerprint: unobserved_fingerprint(),
            payload,
            origin: ClaimOrigin::AssistantInferred,
            initial_status: ClaimStatus::Candidate,
            evidence: Some(ClaimEvidence {
                kind: self.evidence_kind(&qualified),
                session_id: None,
                turn_ordinal: None,
                observed_unix_ms: now_unix_ms(),
            }),
        };
        let Some(store) = self.state_db.as_ref() else {
            return Err(ToolError::QueryFailed);
        };
        propose_op(store, request)
            .await
            .map_err(|_| ToolError::QueryFailed)
            .map(outcome_payload)
    }

    /// `SuccessfulReadQuery` when a succeeded observation touched the proposed
    /// object this turn, else the weaker `RepeatedObservation`.
    fn evidence_kind(&self, qualified: &QualifiedName) -> EvidenceKind {
        match &self.observations {
            Some(log) if log.touched(&qualified.catalog, &qualified.schema, &qualified.object) => {
                TOUCHED
            }
            _ => UNTOUCHED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The definition the model sees and the validator must agree on the allowed
    /// properties and the kind enum — a drift guard, since the two live apart.
    #[test]
    fn definition_and_validator_agree_on_properties_and_kind_enum() {
        validation::assert_definition_matches(&definition::definition());
    }
}
