use super::{Candidate, Consensus, fingerprint, tally};
use saya_types::QueryResult;
use serde_json::{Value, json};

/// Build a `QueryResult` from column names and rows (each row a JSON array).
fn result(columns: &[&str], rows: Vec<Value>) -> QueryResult {
    let row_count = rows.len();
    QueryResult {
        columns: columns.iter().map(|s| s.to_string()).collect(),
        rows,
        row_count,
        truncated: false,
        executed_sql: String::new(),
    }
}

fn candidate(sql: &str, result: Option<QueryResult>) -> Candidate {
    Candidate {
        sql: sql.to_string(),
        result,
    }
}

// ---------------------------------------------------------------------------
// fingerprint rules
// ---------------------------------------------------------------------------

#[test]
fn fp_identical_results_match() {
    let a = result(
        &["id", "name"],
        vec![json!([1, "alice"]), json!([2, "bob"])],
    );
    let b = result(
        &["id", "name"],
        vec![json!([1, "alice"]), json!([2, "bob"])],
    );
    assert_eq!(fingerprint(&a, false), fingerprint(&b, false));
    assert_eq!(fingerprint(&a, true), fingerprint(&b, true));
}

#[test]
fn fp_different_column_names_differ() {
    let a = result(&["id"], vec![json!([1])]);
    let b = result(&["pk"], vec![json!([1])]);
    assert_ne!(fingerprint(&a, false), fingerprint(&b, false));
}

#[test]
fn fp_different_column_count_differ_even_when_empty() {
    let a = result(&["a", "b"], vec![]);
    let b = result(&["a", "b", "c"], vec![]);
    assert_ne!(fingerprint(&a, false), fingerprint(&b, false));
}

#[test]
fn fp_column_order_matters() {
    let a = result(&["a", "b"], vec![json!([1, 2])]);
    let b = result(&["b", "a"], vec![json!([1, 2])]);
    assert_ne!(fingerprint(&a, false), fingerprint(&b, false));
}

#[test]
fn fp_row_order_ignored_when_unordered() {
    let a = result(&["id"], vec![json!([1]), json!([2]), json!([3])]);
    let b = result(&["id"], vec![json!([3]), json!([1]), json!([2])]);
    assert_eq!(fingerprint(&a, false), fingerprint(&b, false));
}

#[test]
fn fp_row_order_matters_when_ordered() {
    let a = result(&["id"], vec![json!([1]), json!([2])]);
    let b = result(&["id"], vec![json!([2]), json!([1])]);
    assert_eq!(fingerprint(&a, false), fingerprint(&b, false));
    assert_ne!(fingerprint(&a, true), fingerprint(&b, true));
}

#[test]
fn fp_null_distinct_from_empty_and_text_null() {
    let nul = result(&["v"], vec![json!([null])]);
    let empty = result(&["v"], vec![json!([""])]);
    let text = result(&["v"], vec![json!(["NULL"])]);
    let fps = [
        fingerprint(&nul, false),
        fingerprint(&empty, false),
        fingerprint(&text, false),
    ];
    assert_ne!(fps[0], fps[1]);
    assert_ne!(fps[0], fps[2]);
    assert_ne!(fps[1], fps[2]);
    // Reproducible for the same value.
    assert_eq!(
        fingerprint(&nul, false),
        fingerprint(&result(&["v"], vec![json!([null])]), false)
    );
}

#[test]
fn fp_integral_float_equals_integer() {
    let float = result(&["v"], vec![json!([3.0])]);
    let int = result(&["v"], vec![json!([3])]);
    assert_eq!(fingerprint(&float, false), fingerprint(&int, false));
    // Not a constant: a different value differs.
    let four = result(&["v"], vec![json!([4])]);
    assert_ne!(fingerprint(&float, false), fingerprint(&four, false));
}

#[test]
fn fp_negative_integral_float_equals_integer() {
    let float = result(&["v"], vec![json!([-2.0])]);
    let int = result(&["v"], vec![json!([-2])]);
    assert_eq!(fingerprint(&float, false), fingerprint(&int, false));
}

#[test]
fn fp_nonintegral_float_keeps_precision() {
    let a = result(&["v"], vec![json!([3.5])]);
    let b = result(&["v"], vec![json!([3])]);
    assert_ne!(fingerprint(&a, false), fingerprint(&b, false));
    let same = result(&["v"], vec![json!([3.5])]);
    assert_eq!(fingerprint(&a, false), fingerprint(&same, false));
}

#[test]
fn fp_booleans_normalise_and_differ() {
    let t = result(&["v"], vec![json!([true])]);
    let f = result(&["v"], vec![json!([false])]);
    assert_ne!(fingerprint(&t, false), fingerprint(&f, false));
    assert_eq!(
        fingerprint(&t, false),
        fingerprint(&result(&["v"], vec![json!([true])]), false)
    );
}

#[test]
fn fp_whitespace_is_preserved() {
    let padded = result(&["v"], vec![json!([" hi "])]);
    let bare = result(&["v"], vec![json!(["hi"])]);
    assert_ne!(fingerprint(&padded, false), fingerprint(&bare, false));
    assert_eq!(
        fingerprint(&padded, false),
        fingerprint(&result(&["v"], vec![json!([" hi "])]), false)
    );
}

#[test]
fn fp_empty_results_are_consistent() {
    let a = result(&["a"], vec![]);
    let b = result(&["a"], vec![]);
    assert_eq!(fingerprint(&a, false), fingerprint(&b, false));
}

#[test]
fn fp_empty_with_different_columns_differ() {
    let a = result(&["a"], vec![]);
    let b = result(&["b"], vec![]);
    assert_ne!(fingerprint(&a, false), fingerprint(&b, false));
}

#[test]
fn fp_empty_differs_from_nonempty() {
    let empty = result(&["a"], vec![]);
    let full = result(&["a"], vec![json!([1])]);
    assert_ne!(fingerprint(&empty, false), fingerprint(&full, false));
}

// ---------------------------------------------------------------------------
// tally behaviour
// ---------------------------------------------------------------------------

#[test]
fn fp_truncated_differs_from_complete_with_the_same_rows() {
    // A result cut off at the row cap is not the same answer as a complete one
    // holding those rows; grouping them together would let a partial answer
    // borrow a complete one's votes.
    let rows = vec![json!([1, "a"])];
    let complete = result(&["id", "label"], rows.clone());
    let mut cut = result(&["id", "label"], rows);
    cut.truncated = true;
    assert_ne!(fingerprint(&complete, false), fingerprint(&cut, false));
}

#[test]
fn fp_two_truncated_results_with_the_same_rows_match() {
    let mut a = result(&["id"], vec![json!([1])]);
    let mut b = result(&["id"], vec![json!([1])]);
    a.truncated = true;
    b.truncated = true;
    assert_eq!(fingerprint(&a, false), fingerprint(&b, false));
}

#[test]
fn tally_empty_input_does_not_panic() {
    let c = tally(&[], false);
    assert_eq!(
        c,
        Consensus {
            winner: None,
            votes: 0,
            margin: 0,
            tied: false,
            failed: 0,
        }
    );
}

#[test]
fn tally_all_failed() {
    let cands = vec![candidate("a", None), candidate("b", None)];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: None,
            votes: 0,
            margin: 0,
            tied: false,
            failed: 2,
        }
    );
}

#[test]
fn tally_single_successful_wins() {
    let cands = vec![candidate("a", Some(result(&["v"], vec![json!([1])])))];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: Some(0),
            votes: 1,
            margin: 1,
            tied: false,
            failed: 0,
        }
    );
}

#[test]
fn tally_failed_do_not_vote_but_are_counted() {
    let cands = vec![
        candidate("fail", None),
        candidate("ok", Some(result(&["v"], vec![json!([1])]))),
        candidate("fail2", None),
    ];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: Some(1),
            votes: 1,
            margin: 1,
            tied: false,
            failed: 2,
        }
    );
}

#[test]
fn tally_largest_group_wins() {
    let cands = vec![
        candidate("a", Some(result(&["v"], vec![json!([1])]))),
        candidate("b", Some(result(&["v"], vec![json!([1])]))),
        candidate("c", Some(result(&["v"], vec![json!([2])]))),
    ];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: Some(0),
            votes: 2,
            margin: 1,
            tied: false,
            failed: 0,
        }
    );
}

#[test]
fn tally_winner_is_first_member_in_input_order() {
    let cands = vec![
        candidate("c", Some(result(&["v"], vec![json!([2])]))),
        candidate("a", Some(result(&["v"], vec![json!([1])]))),
        candidate("b", Some(result(&["v"], vec![json!([1])]))),
    ];
    let c = tally(&cands, false);
    // Group {[1]} = indices [1, 2]; first member is 1.
    assert_eq!(
        c,
        Consensus {
            winner: Some(1),
            votes: 2,
            margin: 1,
            tied: false,
            failed: 0,
        }
    );
}

#[test]
fn tally_tie_for_lead_is_not_a_win() {
    let cands = vec![
        candidate("a", Some(result(&["v"], vec![json!([1])]))),
        candidate("b", Some(result(&["v"], vec![json!([2])]))),
    ];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: None,
            votes: 1,
            margin: 0,
            tied: true,
            failed: 0,
        }
    );
}

#[test]
fn tally_three_way_tie() {
    let cands = vec![
        candidate("a", Some(result(&["v"], vec![json!([1])]))),
        candidate("b", Some(result(&["v"], vec![json!([2])]))),
        candidate("c", Some(result(&["v"], vec![json!([3])]))),
    ];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: None,
            votes: 1,
            margin: 0,
            tied: true,
            failed: 0,
        }
    );
}

#[test]
fn tally_two_tied_groups_of_two() {
    let cands = vec![
        candidate("a", Some(result(&["v"], vec![json!([1])]))),
        candidate("b", Some(result(&["v"], vec![json!([1])]))),
        candidate("c", Some(result(&["v"], vec![json!([2])]))),
        candidate("d", Some(result(&["v"], vec![json!([2])]))),
    ];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: None,
            votes: 2,
            margin: 0,
            tied: true,
            failed: 0,
        }
    );
}

#[test]
fn tally_ordered_passes_through_to_fingerprint() {
    let cands = vec![
        candidate("a", Some(result(&["v"], vec![json!([1]), json!([2])]))),
        candidate("b", Some(result(&["v"], vec![json!([2]), json!([1])]))),
    ];
    // Unordered: same rows -> agreement, winner is first member (index 0).
    let cu = tally(&cands, false);
    assert_eq!(
        cu,
        Consensus {
            winner: Some(0),
            votes: 2,
            margin: 2,
            tied: false,
            failed: 0,
        }
    );
    // Ordered: row order differs -> two singleton groups -> tie.
    let co = tally(&cands, true);
    assert_eq!(
        co,
        Consensus {
            winner: None,
            votes: 1,
            margin: 0,
            tied: true,
            failed: 0,
        }
    );
}
