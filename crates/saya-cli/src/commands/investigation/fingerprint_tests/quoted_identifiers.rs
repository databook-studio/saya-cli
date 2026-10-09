#[test]
fn quoted_dotted_table_is_one_part_and_resolves() {
    // A2 decision 1: the parts arrive as the SQL wrote them and are never
    // re-split, so `"orders.v1"` is one object that resolves as a whole.
    let tree = tree_of_tables(vec![(
        "data",
        vec![("main", vec![table("orders.v1", &["id"])])],
    )]);
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Sqlite,
        Some(&quoted_references(&[&["orders.v1"]])),
    );
    assert!(
        complete(analysis).is_some(),
        "the dotted name resolves whole"
    );
}

#[test]
fn dotted_name_is_never_split_into_schema_and_table() {
    // The F3 shape: a schema `orders` and a table `v1` exist, but the SQL
    // named the single table `orders.v1` — splitting it must not resolve.
    let tree = tree(vec![("data", vec![("orders", vec!["v1"])])]);
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Sqlite,
        Some(&quoted_references(&[&["orders.v1"]])),
    );
    let reason = unverifiable(analysis);
    assert!(
        reason.contains(r#"table "orders.v1" not found"#),
        "{reason}"
    );
}
