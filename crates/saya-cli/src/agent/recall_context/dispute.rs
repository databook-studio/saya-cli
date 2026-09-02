//! Surfacing contract conflicts in the prompt body — slice 5e.
//!
//! recall (2b-1) already detects conflicts and returns them on the contract;
//! this module turns that typed `Vec<ContractConflict>` into the in-band marks
//! and the per-contract summary the model reads. It consumes detection
//! unchanged — it never drops, ranks, or resolves between conflicting claims.
//!
//! Presentation-only, like the rest of `recall_context`: no claim ids reach the
//! body. The summary names the disputed kind and how many claims dispute it —
//! the same level of detail the claim lines carry, never the bookkeeping.

use crate::contracts::{ContractConflict, RetrievedContract};
use saya_types::ClaimId;
use std::collections::HashSet;
use std::fmt::Write;

/// The fixed, in-band prefix that marks a claim as party to a conflict. Empty
/// for every undisputed claim. Mirrors the authority markers in `render`: a
/// reader scanning claim lines cannot miss it. `claim_line` gives a disputed
/// claim this marker *instead of* `[confirmed] `, so a disagreement never reads
/// as a settled instruction.
pub(super) const DISPUTE_MARKER: &str = "[disputed] ";

/// The ids of every claim that participates in any conflict on this contract.
/// A claim is disputed iff its id is in some `ContractConflict::claim_ids`.
pub(super) fn disputed_ids(conflicts: &[ContractConflict]) -> HashSet<String> {
    conflicts
        .iter()
        .flat_map(|c| c.claim_ids.iter().map(ClaimId::as_str).map(str::to_owned))
        .collect()
}

/// One summary line per conflict plus a single instruction, all appended to
/// the contract's body. Names the kind and the claim count — never the ids —
/// and tells the model not to choose between the disputed claims silently.
pub(super) fn conflict_lines(contract: &RetrievedContract) -> String {
    let mut out = String::new();
    for conflict in &contract.conflicts {
        let _ = writeln!(
            out,
            "  conflict: {kind} is disputed by {n} claims",
            kind = conflict.kind,
            n = conflict.claim_ids.len(),
        );
    }
    if !contract.conflicts.is_empty() {
        let _ = writeln!(
            out,
            "  do not choose between disputed claims silently; use the undisputed claims \
            and say the disputed point is unresolved if it matters to the answer.",
        );
    }
    out
}
