//! Building the P1a [`RecallReceipt`] from the contracts that reached the
//! block. This is the agent-layer glue that populates the contract type in
//! [`crate::contracts::receipt`]; the type itself owns no construction logic.
//!
//! The receipt names what was **supplied** (the claims whose rendered lines
//! reached the context block), never what the model **used**. See the type's
//! own docs for that correctness property.

use std::collections::HashMap;

use crate::contracts::{ContractSchemaState, RetrievedContract, SuppliedClaim, SuppliedContract};

/// Builds the [`SuppliedContract`] entries for the contracts whose rendered
/// stanzas reached the block — `contracts` is already the kept prefix the byte
/// bound admitted. `name_of` carries the human-facing profile name keyed by the
/// opaque identity string; the identity itself never enters the receipt.
pub(super) fn supplied_contracts(
    contracts: &[RetrievedContract],
    name_of: &HashMap<String, String>,
) -> Vec<SuppliedContract> {
    contracts
        .iter()
        .map(|contract| {
            let profile = name_of
                .get(contract.object.profile().as_str())
                .cloned()
                .unwrap_or_default();
            SuppliedContract {
                profile,
                object: contract.object.qualified_name(),
                schema_state: schema_state_token(contract.schema_state),
                claims: contract
                    .claims
                    .iter()
                    .map(|claim| {
                        let (column, value) = super::render::claim_value(&claim.value);
                        SuppliedClaim {
                            claim_id: claim.id.clone(),
                            kind: claim.value.kind(),
                            value,
                            column,
                            status: claim.status,
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

/// The stable token for a contract's aggregated schema state, as the receipt
/// carries it. Mirrors [`super::render::schema_state_token`].
fn schema_state_token(state: ContractSchemaState) -> &'static str {
    use ContractSchemaState as S;
    match state {
        S::Current => "current",
        S::NeedsReview => "needs_review",
        S::LiveSchemaUnavailable => "live_schema_unavailable",
        // A contract that aggregates to `Stale` is dropped by the model-path
        // policy before supply, so this never reaches the receipt. The arm is
        // exhaustive and names the impossible case rather than leaving it open.
        S::Stale => "stale",
    }
}
