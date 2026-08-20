//! Detect when a generated SQL statement contradicts a confirmed claim.
//!
//! Spec P2b-1. Pure: no store, no schema fetch, no clock, no I/O. The caller
//! (P2b-2, not this slice) parses the SQL with [`saya_connectors::sql_references`]
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
mod tests {
    use super::*;
    use crate::contracts::RecallOutcomeKind;
    use saya_connectors::sql_references;
    use saya_types::{ClaimId, ClaimStatus, SqlDialect};

    /// Postgres parses the most syntax, so it is the default dialect here — the
    /// detector reads only names, so the dialect matters only for parsing.
    const D: SqlDialect = SqlDialect::Postgres;

    /// The live regression object and claim: a confirmed `default_time_column`
    /// of `return_date` on `pagila.public.rental`.
    fn rental_return_date_receipt() -> RecallReceipt {
        one_contract_receipt(ClaimStatus::Confirmed, "return_date")
    }

    fn one_contract_receipt(status: ClaimStatus, value: &str) -> RecallReceipt {
        RecallReceipt {
            kind: RecallOutcomeKind::Ran {
                store_unavailable: false,
            },
            supplied: vec![SuppliedContract {
                profile: "pagila".into(),
                object: "pagila.public.rental".into(),
                schema_state: "current",
                claims: vec![SuppliedClaim {
                    claim_id: ClaimId::parse("c-rental-time").unwrap(),
                    kind: "default_time_column",
                    value: value.into(),
                    column: None,
                    status,
                }],
            }],
            dropped_by_bounds: 0,
        }
    }

    fn refs_for(sql: &str) -> Option<SqlReferences> {
        sql_references(sql, D)
    }

    // 1. Confirmed default_time_column = return_date; SQL filters on
    //    rental_date of that table → one finding. This is the live case.
    #[test]
    fn confirmed_time_column_overridden_by_a_different_time_column() {
        let receipt = rental_return_date_receipt();
        let refs = refs_for("SELECT rental_date FROM rental WHERE rental_date > '2024-01-01'");
        let findings = detect_overrides(&receipt, refs.as_ref());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.claim_id, ClaimId::parse("c-rental-time").unwrap());
        assert_eq!(f.kind, "default_time_column");
        assert_eq!(f.claimed_value, "return_date");
        // `rental_date` was referenced where the claim named `return_date`.
        assert_eq!(f.observed_columns, vec!["rental_date".to_string()]);
    }

    /// The statement glm-5.2 actually generated when it overrode the confirmed
    /// claim, byte for byte from a live run against pagila. The simplified SQL
    /// in the test above shares none of its shape: this one wraps the column in
    /// `TO_CHAR`, qualifies the table with all three parts, and repeats the
    /// column in `GROUP BY`. If the extractor ever stops reaching inside a
    /// function call, or stops resolving a 3-part name, the detector goes quiet
    /// on the exact case it exists to catch — and every unit test above would
    /// still pass. Hence this one.
    const LIVE_OVERRIDE_SQL: &str = "SELECT TO_CHAR(rental_date, 'YYYY-MM') AS month, \
         COUNT(*) AS rental_count FROM pagila.public.rental \
         WHERE rental_date >= '2022-01-01' AND rental_date < '2023-01-01' \
         GROUP BY TO_CHAR(rental_date, 'YYYY-MM') ORDER BY month";

    /// The compliant statement from the same live run, for the same reason: the
    /// detector must stay silent when the model honoured the claim.
    const LIVE_COMPLIANT_SQL: &str = "SELECT date_trunc('month', return_date) AS month, \
         COUNT(*) AS rental_count FROM pagila.public.rental \
         WHERE return_date >= '2022-01-01' AND return_date < '2023-01-01' \
         GROUP BY date_trunc('month', return_date) ORDER BY month";

    #[test]
    fn the_real_generated_override_sql_is_detected() {
        let findings = detect_overrides(
            &rental_return_date_receipt(),
            refs_for(LIVE_OVERRIDE_SQL).as_ref(),
        );
        assert_eq!(
            findings.len(),
            1,
            "the SQL from the live override must produce exactly one finding: {findings:?}"
        );
        assert_eq!(findings[0].claimed_value, "return_date");
        assert!(
            findings[0]
                .observed_columns
                .contains(&"rental_date".to_string()),
            "the finding must name the column actually referenced: {findings:?}"
        );
    }

    #[test]
    fn the_real_compliant_sql_is_not_flagged() {
        let findings = detect_overrides(
            &rental_return_date_receipt(),
            refs_for(LIVE_COMPLIANT_SQL).as_ref(),
        );
        assert!(
            findings.is_empty(),
            "honouring the claim must never be reported as an override: {findings:?}"
        );
    }

    // 2. Same claim; SQL filters on return_date → no finding.
    #[test]
    fn claimed_column_used_is_no_override() {
        let receipt = rental_return_date_receipt();
        let refs = refs_for("SELECT return_date FROM rental WHERE return_date > '2024-01-01'");
        assert!(detect_overrides(&receipt, refs.as_ref()).is_empty());
    }

    // 3. Same claim; SQL references the table but no time column → no finding
    //    (absence ≠ contradiction). `amount` is not time-named.
    #[test]
    fn non_time_column_does_not_contradict() {
        let receipt = rental_return_date_receipt();
        let refs = refs_for("SELECT amount FROM rental");
        assert!(detect_overrides(&receipt, refs.as_ref()).is_empty());
    }

    // 4. Candidate (not confirmed) claim contradicted → no finding. The status
    //    guard is asserted directly: only a Confirmed claim can be overridden.
    #[test]
    fn candidate_claim_is_not_overridable() {
        let receipt = one_contract_receipt(ClaimStatus::Candidate, "return_date");
        let refs = refs_for("SELECT rental_date FROM rental WHERE rental_date > '2024-01-01'");
        // A candidate is advisory; contradicting it is not a fault. Even though
        // the SQL uses a different time column, no finding is produced.
        assert!(detect_overrides(&receipt, refs.as_ref()).is_empty());
        // Direct assertion of the guard: a confirmed claim under the same SQL
        // does fire, so the candidate path is what suppresses the finding.
        let confirmed = rental_return_date_receipt();
        assert_eq!(
            detect_overrides(&confirmed, refs.as_ref()).len(),
            1,
            "the same SQL must fire for a confirmed claim, proving the status guard is the gate"
        );
    }

    // 5. sql_references returns None → no findings, never a guess.
    #[test]
    fn unparseable_sql_yields_no_findings() {
        let receipt = rental_return_date_receipt();
        let refs = refs_for("SELECT FROM WHERE");
        assert!(refs.is_none(), "fixture must be unparseable");
        assert!(detect_overrides(&receipt, None).is_empty());
    }

    // 6. partial == true → no findings. Under partial the column list is "at
    //    least these, possibly more"; the claimed column's absence is not
    //    reliable, so the contradiction (which rests on that absence) is not
    //    asserted. Fail closed.
    #[test]
    fn partial_column_list_yields_no_findings() {
        let receipt = rental_return_date_receipt();
        // Built by hand so the partial branch is exercised directly and the
        // fixture is deterministic: `rental` is referenced, `rental_date` is a
        // different time-named column, and `return_date` is absent — every leg
        // of the contradiction holds except the partial flag, which suppresses
        // the finding. Asserting partial is not asserting the test's outcome; it
        // is asserting the fixture reaches the branch under test.
        let refs = SqlReferences {
            objects: vec![vec!["rental".to_string()]],
            columns: vec!["rental_date".to_string()],
            partial: true,
        };
        assert!(refs.partial, "fixture must be partial");
        assert!(
            detect_overrides(&receipt, Some(&refs)).is_empty(),
            "under partial, the claimed column's absence is not reliable → no finding"
        );
    }

    // 7. SQL references a different table entirely → no finding.
    #[test]
    fn different_object_is_no_override() {
        let receipt = rental_return_date_receipt();
        // `film` is not the supplied `rental`; even with a time-named column,
        // the object-match gate suppresses the finding.
        let refs = refs_for("SELECT rental_date FROM film");
        assert!(detect_overrides(&receipt, refs.as_ref()).is_empty());
    }

    // 8. Determinism: same inputs, same findings, same order.
    #[test]
    fn same_inputs_same_findings_same_order() {
        let receipt = rental_return_date_receipt();
        let refs = refs_for("SELECT rental_date FROM rental WHERE rental_date > '2024-01-01'");
        let a = detect_overrides(&receipt, refs.as_ref());
        let b = detect_overrides(&receipt, refs.as_ref());
        assert_eq!(a, b);
        // Observed columns are sorted, so repeated inputs pick the same order.
        let refs2 = refs_for("SELECT rental_date FROM rental");
        assert_eq!(
            detect_overrides(&receipt, refs2.as_ref()),
            detect_overrides(&receipt, refs2.as_ref())
        );
    }

    // --- Extra: the binding invariants the spec asks to assert directly. ---

    /// Only `default_time_column` is handled; every other kind returns nothing
    /// even when the SQL plainly contradicts its value. Stated for §4.
    #[test]
    fn only_default_time_column_is_handled() {
        // A table_alias claim of `r` contradicted by an alias of `x` → no
        // finding: aliases are advisory and a name-only contradiction is a guess.
        let receipt = RecallReceipt {
            kind: RecallOutcomeKind::Ran {
                store_unavailable: false,
            },
            supplied: vec![SuppliedContract {
                profile: "pagila".into(),
                object: "pagila.public.rental".into(),
                schema_state: "current",
                claims: vec![SuppliedClaim {
                    claim_id: ClaimId::parse("c-alias").unwrap(),
                    kind: "table_alias",
                    value: "r".into(),
                    column: None,
                    status: ClaimStatus::Confirmed,
                }],
            }],
            dropped_by_bounds: 0,
        };
        let refs = refs_for("SELECT * FROM rental AS x");
        assert!(detect_overrides(&receipt, refs.as_ref()).is_empty());
    }

    /// A multi-table statement returns no finding: a flat column list cannot be
    /// attributed to one table, and guessing the attribution risks a false
    /// accusation. Fail closed (spec §3 + the governing rule).
    #[test]
    fn multi_table_statement_yields_no_findings() {
        let receipt = rental_return_date_receipt();
        // `rental_date` may belong to `payment`, not `rental`; we do not guess.
        let refs = refs_for(
            "SELECT rental_date FROM rental JOIN payment ON rental.payment_id = payment.id",
        );
        assert!(refs.as_ref().map(|r| r.objects.len()).unwrap_or(0) > 1);
        assert!(detect_overrides(&receipt, refs.as_ref()).is_empty());
    }

    /// An ambiguous bare object name (two supplied rentals in different
    /// schemas) matches neither — a bare `rental` is not assumed to be either.
    #[test]
    fn ambiguous_bare_object_matches_neither() {
        let receipt = RecallReceipt {
            kind: RecallOutcomeKind::Ran {
                store_unavailable: false,
            },
            supplied: vec![
                SuppliedContract {
                    profile: "pagila".into(),
                    object: "pagila.public.rental".into(),
                    schema_state: "current",
                    claims: vec![SuppliedClaim {
                        claim_id: ClaimId::parse("c-a").unwrap(),
                        kind: "default_time_column",
                        value: "return_date".into(),
                        column: None,
                        status: ClaimStatus::Confirmed,
                    }],
                },
                SuppliedContract {
                    profile: "archive".into(),
                    object: "archive.public.rental".into(),
                    schema_state: "current",
                    claims: vec![SuppliedClaim {
                        claim_id: ClaimId::parse("c-b").unwrap(),
                        kind: "default_time_column",
                        value: "return_date".into(),
                        column: None,
                        status: ClaimStatus::Confirmed,
                    }],
                },
            ],
            dropped_by_bounds: 0,
        };
        // Bare `rental` is consistent with both supplied objects → ambiguous.
        let refs = refs_for("SELECT rental_date FROM rental");
        assert!(detect_overrides(&receipt, refs.as_ref()).is_empty());
    }

    /// A disqualifying qualifier refuses the match: `warehouse.rental` is not
    /// the supplied `pagila.public.rental`, even though the object segment
    /// matches. Spec §3: a bare name is not assumed; a contradictory qualifier
    /// is a different object.
    #[test]
    fn disagreeing_schema_qualifier_is_no_match() {
        let receipt = rental_return_date_receipt();
        let refs = refs_for("SELECT rental_date FROM warehouse.rental");
        assert!(detect_overrides(&receipt, refs.as_ref()).is_empty());
    }

    /// Case folding prevents a false accusation: a claim stored as
    /// `Return_Date` and SQL `return_date` are the same column, so the claimed
    /// column is present and no finding fires (folding only turns misses into
    /// matches, never the reverse).
    #[test]
    fn case_fold_keeps_claimed_column_match() {
        let receipt = one_contract_receipt(ClaimStatus::Confirmed, "Return_Date");
        let refs = refs_for("SELECT return_date FROM rental WHERE return_date > '2024-01-01'");
        assert!(detect_overrides(&receipt, refs.as_ref()).is_empty());
    }
}
