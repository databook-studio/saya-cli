#[test]
fn missing_table_is_unverifiable_not_a_constant() {
    // A2 decision 4: a missing object makes the review unverifiable — it can
    // never again contribute a constant digest that survives table changes.
    let analysis = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[&["events"], &["ghost"]])),
    );
    let reason = unverifiable(analysis);
    assert!(reason.contains(r#"table "ghost" not found"#), "{reason}");
}

#[test]
fn missing_and_ambiguous_problems_are_named_together() {
    let tree = tree(vec![(
        "data",
        vec![("main", vec!["t"]), ("archive", vec!["t"])],
    )]);
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Sqlite,
        Some(&references(&[&["ghost"], &["t"]])),
    );
    let reason = unverifiable(analysis);
    assert!(reason.contains(r#"table "ghost" not found"#), "{reason}");
    assert!(reason.contains(r#"ambiguous name "t""#), "{reason}");
}

#[test]
fn same_unqualified_name_in_two_schemas_is_unverifiable() {
    let tree = tree(vec![(
        "data",
        vec![("main", vec!["t"]), ("archive", vec!["t"])],
    )]);
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Sqlite,
        Some(&references(&[&["t"]])),
    );
    let reason = unverifiable(analysis);
    assert!(reason.contains(r#"ambiguous name "t""#), "{reason}");
    // A qualified name still resolves exactly within its schema.
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Sqlite,
        Some(&references(&[&["archive", "t"]])),
    );
    assert!(complete(analysis).is_some());
}
