//! Conflict detection: active items of an exclusive kind that disagree.
//!
//! A single-valued slot has one current value (enforced by the
//! `knowledge_items` cardinality at the storage boundary), so two *admitted*
//! items of a single-valued kind on one object can only arise across the
//! `Active`/`Pending` divide or a stale write. Per the SPEC REVIEW, only
//! `table_grain` is treated as exclusive: a table has one grain.
//! `TableDescription` is NOT exclusive — a table can be described from two
//! angles without contradiction, and the store deliberately keeps distinct
//! descriptions.

use crate::contracts::view::{ContractClaim, ContractConflict};
use saya_types::{ClaimId, ClaimStatus};

/// Returns the conflicts among `claims` (the admitted claims of one object).
/// Only `table_grain` is exclusive today; any other kind that should be
/// exclusive is a store-level cardinality concern, not a read-time conflict.
pub(crate) fn conflicts_for(claims: &[ContractClaim]) -> Vec<ContractConflict> {
    let grains: Vec<ClaimId> = claims
        .iter()
        .filter(|c| c.status == ClaimStatus::Confirmed && kind_is(c, "table_grain"))
        .map(|c| c.id.clone())
        .collect();
    if grains.len() > 1 {
        vec![ContractConflict {
            kind: "table_grain",
            claim_ids: grains,
        }]
    } else {
        Vec::new()
    }
}

fn kind_is(claim: &ContractClaim, expected: &str) -> bool {
    claim.value.kind() == expected
}
