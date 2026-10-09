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
    let refs =
        refs_for("SELECT rental_date FROM rental JOIN payment ON rental.payment_id = payment.id");
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
