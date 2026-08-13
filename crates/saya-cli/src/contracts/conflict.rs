//! Conflict detection: confirmed claims of an exclusive kind that disagree.
//!
//! The store's deduplication key already prevents duplicate `ColumnRole`,
//! `DefaultTimeColumn`, and identical-text `TableAlias`/`TableDescription`/
//! `TableGrain` claims at write time. What survives that gate and can still
//! disagree is more than one confirmed claim of a kind that is exclusive per
//! object. Per the SPEC REVIEW, only `TableGrain` is treated as exclusive: a
//! table has one grain. `TableDescription` is NOT exclusive — a table can be
//! described from two angles without contradiction, and the store deliberately
//! keeps distinct descriptions (its dedup key hashes the text).

use crate::contracts::view::ContractConflict;
use saya_store::StoredClaim;
use saya_types::ClaimId;

/// Returns the conflicts among `claims` (the recallable claims of one object).
/// Only `table_grain` is exclusive today; any other kind that should be
/// exclusive is a store-level dedup concern, not a read-time conflict.
pub(crate) fn conflicts_for(claims: &[StoredClaim]) -> Vec<ContractConflict> {
    let grains: Vec<ClaimId> = claims
        .iter()
        .filter(|c| c.status.is_recallable() && kind_is(c, "table_grain"))
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

fn kind_is(claim: &StoredClaim, expected: &str) -> bool {
    claim.payload.as_ref().map(|p| p.kind()) == Some(expected)
}
