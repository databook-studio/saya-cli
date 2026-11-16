#[test]
fn partial_analysis_is_unverifiable() {
    let analysis = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&partial_references(&[&["events"]])),
    );
    assert_eq!(unverifiable(analysis), "dependency analysis incomplete");
    assert!(
        !needs_schema(Some(&partial_references(&[&["events"]]))),
        "an incomplete analysis never fetches the schema"
    );
}

#[test]
fn absent_analysis_is_unverifiable() {
    let analysis = analyze(Some(&sqlite_tree()), SqlDialect::Sqlite, None);
    assert_eq!(unverifiable(analysis), "dependency analysis incomplete");
    assert!(!needs_schema(None));
}
