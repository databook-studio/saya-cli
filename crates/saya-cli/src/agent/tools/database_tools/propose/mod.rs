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
mod log;
mod mapping;
mod validation;

pub(crate) use definition::definition as propose_definition;
pub(crate) use log::ProposedClaimsLog;

use saya_agent::{ProposedClaimDto, ToolError};
use saya_store::{ClaimEvidence, EvidenceKind, ProposeClaim, ProposeOutcome};
use saya_types::{
    ClaimOrigin, ClaimStatus, DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity,
};

use super::DatabaseTools;
use crate::agent::recall_context::claim_value;
use crate::commands::unobserved_fingerprint;
use crate::contracts::args::{QualifiedName, build_payload, parse_kind, parse_qualified};
use crate::contracts::propose as propose_op;

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
        // The profile *name* the event carries (never the opaque identity). The
        // connection the model named, or the primary when it left `connection`
        // blank — the same resolution `registry.resolve` just validated.
        let profile_name = args
            .connection
            .filter(|name| !name.is_empty())
            .unwrap_or(self.registry.primary_name())
            .to_string();
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

        // Pre-compute the `KnowledgeProposed` fields before `payload`/`object`
        // move into the store request. `claim_value` is the recall render path's
        // single source, so the event names the same value a later recall would.
        let object_qualified = object.qualified_name();
        let kind_str = payload.kind().to_string();
        let (claim_column, claim_value) = claim_value(&payload);

        let referenced_columns = payload.referenced_column_name_snapshots();
        let request = ProposeClaim {
            object,
            fingerprint: unobserved_fingerprint(),
            // The agent proposal path has no live schema in hand, so it records
            // referenced column names without claiming a type — the same
            // unknown treatment the headless `remember` path uses.
            referenced_columns,
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
        let outcome = propose_op(store, request)
            .await
            .map_err(|_| ToolError::QueryFailed)?;
        // Record only what was *persisted*. A `Stored` outcome is a new
        // candidate; a `Duplicate` (live or forgotten) is not a new proposal, and
        // the `Err` arms above already returned. So the event's data is captured
        // here and only here — on the write, not on the tool call (spec P2d §3).
        // The runtime drains the log after the turn and emits one
        // `KnowledgeProposed` per recorded claim; recording never fails the turn
        // or rolls back the write, and a missing log (tests without one) is a
        // no-op.
        if let ProposeOutcome::Stored(claim_id) = &outcome
            && let Some(log) = self.proposed_claims.as_ref()
        {
            log.record(ProposedClaimDto {
                claim_id: claim_id.clone(),
                profile: profile_name,
                object: object_qualified,
                kind: kind_str,
                value: claim_value,
                column: claim_column,
                // A proposal is always a candidate — inert until a human
                // confirms it — so the event never reads as established.
                status: ClaimStatus::Candidate,
            });
        }
        Ok(outcome_payload(outcome))
    }

    /// `SuccessfulReadQuery` when a succeeded observation touched the proposed
    /// object this turn AND no claims for this object were supplied this turn;
    /// else the weaker `RepeatedObservation`.
    pub(crate) fn evidence_kind(&self, qualified: &QualifiedName) -> EvidenceKind {
        let proposed = format!(
            "{}.{}.{}",
            qualified.catalog, qualified.schema, qualified.object
        );
        let supplied = evidence::is_supplied_object(&self.supplied_objects, &proposed);
        let touched = match &self.observations {
            Some(log) => log.touched(&qualified.catalog, &qualified.schema, &qualified.object),
            None => false,
        };
        evidence::decide_evidence_kind(touched, supplied)
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
