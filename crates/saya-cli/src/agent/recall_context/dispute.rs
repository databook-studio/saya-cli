//! Surfacing contract conflicts in the prompt body — slice 5e.
//!
//! recall (2b-1) already detects conflicts and returns them on the contract;
//! this module turns that typed `Vec<ContractConflict>` into the in-band marks
//! and the per-contract summary the model reads. It consumes detection
//! unchanged — it never drops, ranks, or resolves between conflicting claims
//! (spec 5e §2, ADR 0002 §3).
//!
//! Presentation-only, like the rest of `recall_context`: no claim ids reach the
//! body. The summary names the disputed kind and how many claims dispute it —
//! the same level of detail the claim lines carry, never the bookkeeping.

use crate::contracts::{ContractConflict, RetrievedContract};
use saya_types::ClaimId;
use std::collections::HashSet;
use std::fmt::Write;

/// The fixed, in-band prefix that marks a claim as party to a conflict. Empty
/// for every undisputed claim, so a contract with no conflict renders
/// byte-identically to before this slice. Mirrors the candidate marker in
/// `render`: a reader scanning claim lines cannot miss it.
pub(super) const DISPUTE_MARKER: &str = "[disputed] ";

/// The ids of every claim that participates in any conflict on this contract.
/// A claim is disputed iff its id is in some `ContractConflict::claim_ids`.
pub(super) fn disputed_ids(conflicts: &[ContractConflict]) -> HashSet<String> {
    conflicts
        .iter()
        .flat_map(|c| c.claim_ids.iter().map(ClaimId::as_str).map(str::to_owned))
        .collect()
}

/// The in-band prefix for a claim, or empty. `is_disputed` is precomputed by the
/// caller so this stays a trivial branch the renderer can compose with the
/// candidate marker.
pub(super) fn dispute_marker(is_disputed: bool) -> &'static str {
    if is_disputed { DISPUTE_MARKER } else { "" }
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
