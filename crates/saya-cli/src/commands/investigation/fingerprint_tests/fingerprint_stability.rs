#[test]
fn zero_objects_complete_without_a_fingerprint() {
    let analysis = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[])),
    );
    assert_eq!(complete(analysis), None);
    assert!(!needs_schema(Some(&references(&[]))), "no schema is needed");
}

#[test]
fn fingerprint_is_order_independent_over_objects() {
    let sorted = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[&["events"], &["orders"]])),
    );
    let shuffled = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[&["orders"], &["events"]])),
    );
    assert_eq!(complete(sorted), complete(shuffled));
}

#[test]
fn fingerprint_changes_when_a_column_is_added() {
    let before = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[&["events"]])),
    );
    let mut changed = sqlite_tree();
    changed.databases[0].schemas[0].tables[0]
        .columns
        .push(Column {
            name: "extra".into(),
            data_type: "text".into(),
            nullable: true,
        });
    let after = analyze(
        Some(&changed),
        SqlDialect::Sqlite,
        Some(&references(&[&["events"]])),
    );
    assert_ne!(
        complete(before),
        complete(after),
        "an added column invalidates the review"
    );
}
