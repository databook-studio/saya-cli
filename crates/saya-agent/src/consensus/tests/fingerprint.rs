use super::support::result;
use crate::consensus::fingerprint;
use serde_json::json;

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
