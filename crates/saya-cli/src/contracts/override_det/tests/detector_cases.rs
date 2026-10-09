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
        object_quoting: vec![vec![false]],
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
