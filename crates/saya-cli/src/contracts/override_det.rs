//! Detect when a generated SQL statement contradicts a confirmed claim.
//!
//! Spec P2b-1. Pure: no store, no schema fetch, no clock, no I/O. The caller
//! Override detection parses the SQL with [`saya_connectors::sql_references`]
//! and passes the result here; this function only reads the names.
//!
//! ## What a contradiction is, and the failure direction
//!
//! A confirmed `default_time_column` claim names *the* time column for its
//! object. The SQL contradicts it when, on that object, it references a
//! *different* time-named column and not the claimed one — the model reached
//! for another time column where the contract named one. The governing rule
//! (spec §3) is that a false accusation is worse than a miss, so this detector
//! fails closed at every doubt: an unparseable statement, a `partial` column
//! list, an ambiguous object reference, and a multi-table statement all return
//! no finding rather than guess.
//!
//! ## What a finding asserts (spec §6)
//!
//! A finding asserts that the claim was contradicted and names the time-named
//! columns the SQL *referenced* instead — observed references, not "the time
//! column SAYA used." From object and column names alone the role of a column
//! (predicate vs projection) is unknowable, so the finding stops at "these were
//! referenced where the claim named a different column." A later slice may render
//! that as a true sentence; it may not render "SAYA used X as the time column,"
//! which the names do not prove.

use saya_connectors::SqlReferences;
use saya_types::{ClaimId, ClaimStatus};

use super::receipt::{RecallReceipt, SuppliedClaim, SuppliedContract};

/// A confirmed claim the SQL contradicts, with what was observed. One finding
/// per contradicted confirmed claim (spec §2: "the confirmed claims …").
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OverrideFinding {
    pub claim_id: ClaimId,
    /// The claim kind. Only `default_time_column` is ever produced here; every
    /// other kind is deliberately not detected (see [`detect_overrides`]).
    pub kind: &'static str,
    /// The value the claim specifies — for `default_time_column`, the claimed
    /// time column. Carried so a render can say "where you specified Y".
    pub claimed_value: String,
    /// Time-named columns the SQL referenced instead, as written, sorted for
    /// determinism. **Observed references**, not an asserted "used" column: the
    /// finding says these were referenced where the claim named a different one.
    pub observed_columns: Vec<String>,
}

/// The claim kinds this detector handles. Only `default_time_column` has a
/// contradiction checkable from object and column names alone — a different
/// time-named column referenced where the claim names one. Every other kind
/// (`table_alias`, `table_description`, `table_grain`, `column_description`,
/// `column_role`, `relationship`, and any future variant) returns no finding:
/// their contradictions are not name-only (a role, a grain, a description, a
/// join shape), and approximating them would guess — the failure §3 forbids.
pub(crate) const HANDLED_KIND: &str = "default_time_column";

/// Returns the confirmed claims the SQL contradicts. `refs` is `None` when the
/// statement did not parse (the caller's `sql_references` returned `None`); an
/// unparseable statement yields no findings, never a guess (spec §3).
///
/// Same inputs → same output, in a stable order (supplied-claim order, observed
/// columns sorted). Nothing else is read.
pub(crate) fn detect_overrides(
    receipt: &RecallReceipt,
    refs: Option<&SqlReferences>,
) -> Vec<OverrideFinding> {
    let Some(refs) = refs else {
        // Unparseable SQL: the only honest answer is no findings. Inferring an
        // override from input we could not parse is exactly the guess §3 bars.
        return Vec::new();
    };
    if refs.partial {
        // `partial` means "at least these columns, possibly more." A
        // `default_time_column` contradiction rests on the *claimed* column being
        // absent from the column list; under `partial` that absence is not
        // reliable (the claimed column may be in the un-extracted tail). Fail
        // closed. The positively-observed columns stay true under `partial`, but
        // they only matter alongside an absence we cannot establish.
        return Vec::new();
    }
    // The flat column list is attributable to the claim's object only when the
    // statement references exactly one object. With a join, a time-named column
    // may belong to the other table; attributing it to this claim's object would
    // guess, and a wrong guess is a false accusation. Fail closed for >1 object.
    let Some((object, columns)) = single_object_table(refs) else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for contract in &receipt.supplied {
        // An object reference matches a supplied contract only when it resolves
        // to exactly one — a bare name consistent with two supplied objects is
        // ambiguous, and accusing either is a guess. See [`object_matches`].
        if !object_matches(object, contract, &receipt.supplied) {
            continue;
        }
        for claim in &contract.claims {
            if let Some(finding) = contradicted_default_time_column(claim, columns) {
                findings.push(finding);
            }
        }
    }
    findings
}

/// The single object a statement references, with its column list, when the
/// statement names exactly one object. More than one → `None` (fail closed).
fn single_object_table(refs: &SqlReferences) -> Option<(&[String], &[String])> {
    if refs.objects.len() == 1 {
        Some((refs.objects[0].as_slice(), refs.columns.as_slice()))
    } else {
        None
    }
}

/// True when `object` (a referenced name, parts as written) resolves to exactly
/// `contract` among `supplied`. The rule (spec §3):
///
/// - Align `object`'s parts as a *suffix* of the contract's `catalog.schema.
///   object`, case-folded. The SQL may omit leading parts (`rental`,
///   `public.rental`) but every part it *did* write must equal the contract's
///   corresponding part — a qualifier that disagrees (`warehouse.rental` vs
///   `pagila.public.rental`) is a different object, not an assumption.
/// - A reference that suffix-matches *two* supplied contracts is ambiguous
///   (two rentals in different schemas, bare name); it matches neither. This
///   is the §3 direction: a bare `rental` is not assumed to be any particular
///   `catalog.schema.rental` when another could match too.
fn object_matches(
    object: &[String],
    contract: &SuppliedContract,
    supplied: &[SuppliedContract],
) -> bool {
    // Count how many supplied contracts this reference suffix-matches. A bare
    // name consistent with more than one is ambiguous → match none.
    let matches: Vec<&SuppliedContract> = supplied
        .iter()
        .filter(|c| suffix_aligns(object, &c.object))
        .collect();
    matches.len() == 1 && matches[0].object == contract.object
}

/// True when `object`'s parts, aligned as a suffix of the contract's qualified
/// `catalog.schema.object` name, agree on every part the SQL wrote. The SQL may
/// omit leading parts; it may not contradict them. Case is folded (ASCII) so a
/// claim stored as `Return_Date` and SQL `return_date` match — folding can only
/// turn a would-be-miss into a match, never invent a false one.
fn suffix_aligns(object: &[String], qualified: &str) -> bool {
    let parts = name_parts(qualified);
    if object.len() > parts.len() {
        return false;
    }
    let offset = parts.len() - object.len();
    object
        .iter()
        .zip(parts[offset..].iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Splits a `catalog.schema.object` qualified name into its parts. `validate_name`
/// permits a `.` inside a part, so the split is ambiguous in that edge case; the
/// extractor splits SQL names the same way, so a mismatch fails closed (no finding).
fn name_parts(qualified: &str) -> Vec<String> {
    qualified.split('.').map(String::from).collect()
}

/// Returns a finding when `claim` is a confirmed `default_time_column` the SQL
/// contradicts: a different time-named column is referenced and the claimed
/// column is not.
fn contradicted_default_time_column(
    claim: &SuppliedClaim,
    columns: &[String],
) -> Option<OverrideFinding> {
    if claim.status != ClaimStatus::Confirmed {
        // Only a confirmed claim binds (spec §3). A candidate is advisory;
        // contradicting one is not a fault. This guard is the direct assertion.
        return None;
    }
    if claim.kind != HANDLED_KIND {
        // Every kind except `default_time_column` is deliberately not detected
        // (see the module docs). Return nothing rather than approximate.
        return None;
    }
    let claimed = claim.value.as_str();
    if claimed.is_empty() {
        // No claimed value to contradict. `claim_value` renders the column into
        // `value` for this kind, so empty means a malformed receipt row.
        return None;
    }
    // The claimed column present → the SQL honors the claim (or at least does
    // not visibly contradict it); absence of a *different* time column then
    // proves nothing. Firing when the claimed column is also referenced would
    // accuse a query that may be honoring the claim. Require the claimed column
    // to be absent.
    let claimed_referenced = columns.iter().any(|c| c.eq_ignore_ascii_case(claimed));
    if claimed_referenced {
        return None;
    }
    // A different time-named column was referenced. These are positively
    // observed, so naming them is honest even though their role is not known.
    let mut observed: Vec<String> = columns
        .iter()
        .filter(|c| !c.eq_ignore_ascii_case(claimed) && is_time_named(c))
        .cloned()
        .collect();
    if observed.is_empty() {
        // No time-named column at all → the query needs no time predicate, so
        // it does not contradict the claim (absence ≠ contradiction, spec §3).
        return None;
    }
    observed.sort();
    observed.dedup();
    Some(OverrideFinding {
        claim_id: claim.claim_id.clone(),
        kind: claim.kind,
        claimed_value: claim.value.clone(),
        observed_columns: observed,
    })
}

/// A column is a *time-named* candidate when its name matches a conventional
/// temporal form: exactly `date` / `time` / `timestamp`, or ending `_date`,
/// `_time`, `_timestamp`, `_at`, `_ts`, `_dt`. This is a name classification, not
/// a guess about the model's intent — it identifies columns that *by naming
/// convention* are temporal, the same way the claim's own value (`return_date`)
/// is. The leading underscore guards against bare-suffix false matches like
/// `update` / `candidate` / `format` ending in `date` / `at`.
fn is_time_named(column: &str) -> bool {
    const EXACT: &[&str] = &["date", "time", "timestamp"];
    const SUFFIX: &[&str] = &["_date", "_time", "_timestamp", "_at", "_ts", "_dt"];
    let lower = column.to_ascii_lowercase();
    if EXACT.contains(&lower.as_str()) {
        return true;
    }
    SUFFIX.iter().any(|s| lower.ends_with(s))
}

#[cfg(test)]
#[path = "override_det/tests.rs"]
mod tests;
