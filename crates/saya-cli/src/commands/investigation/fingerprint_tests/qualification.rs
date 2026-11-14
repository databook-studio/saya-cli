#[test]
fn one_part_name_resolves_anywhere_in_the_tree() {
    let analysis = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[&["events"]])),
    );
    assert!(
        complete(analysis).is_some(),
        "sqlite names tables flatly, so a complete review binds a fingerprint"
    );
}

#[test]
fn two_part_name_resolves_by_schema_across_databases() {
    let postgres_tree = tree(vec![("analytics", vec![("public", vec!["events"])])]);
    let analysis = analyze(
        Some(&postgres_tree),
        SqlDialect::Postgres,
        Some(&references(&[&["public", "events"]])),
    );
    assert!(
        complete(analysis).is_some(),
        "schema-qualified name resolves"
    );
}

#[test]
fn three_part_name_resolves_exactly() {
    let analysis = analyze(
        Some(&postgres_tree()),
        SqlDialect::Postgres,
        Some(&references(&[&["analytics", "public", "events"]])),
    );
    assert!(
        complete(analysis).is_some(),
        "fully-qualified name resolves"
    );
}

#[test]
fn wrong_qualifier_is_missing_not_found() {
    // The table exists, but not under the named schema: the review is
    // unverifiable, never silently resolved somewhere else.
    let analysis = analyze(
        Some(&postgres_tree()),
        SqlDialect::Postgres,
        Some(&references(&[&["analytics", "private", "events"]])),
    );
    let reason = unverifiable(analysis);
    assert!(
        reason.contains(r#"table "analytics.private.events" not found"#),
        "{reason}"
    );
}
