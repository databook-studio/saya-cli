//! The extractor's bounds: truncation at `MAX_OBJECTS`/`MAX_COLUMNS` and
//! the `partial` flag for constructs the extractor does not model.

use super::*;

#[test]
fn object_bound_truncates_and_sets_partial() {
    // Spec case 11: more than 64 objects → partial, truncated to 64.
    let mut from = String::from("SELECT 1 FROM ");
    for i in 0..70 {
        if i > 0 {
            from.push_str(", ");
        }
        from.push_str(&format!("t{i}"));
    }
    let r = sql_references(&from, D).unwrap();
    assert!(r.partial);
    assert_eq!(r.objects.len(), 64);
}

#[test]
fn column_bound_truncates_and_sets_partial() {
    // Spec case 11 (columns): more than 256 columns → partial, truncated.
    let mut select = String::from("SELECT ");
    for i in 0..300 {
        if i > 0 {
            select.push_str(", ");
        }
        select.push_str(&format!("c{i}"));
    }
    select.push_str(" FROM t");
    let r = sql_references(&select, D).unwrap();
    assert!(r.partial);
    assert_eq!(r.columns.len(), 256);
}

#[test]
fn table_valued_function_is_not_an_object_but_flags_partial() {
    // generate_series(...) parses as a table factor with function args; its
    // name is a function, not a table, so it must not be reported as an
    // object and the result is partial (a real object may be hidden).
    let r = sql_references("SELECT * FROM generate_series(1, 10)", D).unwrap();
    assert!(
        r.objects.is_empty()
            || !r
                .objects
                .iter()
                .any(|o| o == &vec!["generate_series".to_string()])
    );
    assert!(r.partial, "a table-valued function must set partial");
}
