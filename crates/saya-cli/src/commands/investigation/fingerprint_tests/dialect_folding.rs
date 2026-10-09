#[test]
fn postgres_unquoted_orders_fingerprints_the_lowercase_table() {
    // Re-audit R1: unquoted `Orders` folds to `orders`, so the review binds
    // the real `orders` — changing `orders` stales the review, changing the
    // quoted `Orders` table does not.
    let plain = complete(analyze(
        Some(&orders_tree()),
        SqlDialect::Postgres,
        Some(&references(&[&["Orders"]])),
    ))
    .expect("unquoted `Orders` folds to the existing `orders`");
    let mut staled = orders_tree();
    stale_column(&mut staled, 0);
    let after = complete(analyze(
        Some(&staled),
        SqlDialect::Postgres,
        Some(&references(&[&["Orders"]])),
    ))
    .expect("still resolves after the change");
    assert_ne!(plain, after, "changing `orders` must stale the review");
    let mut staled = orders_tree();
    stale_column(&mut staled, 1);
    let after = complete(analyze(
        Some(&staled),
        SqlDialect::Postgres,
        Some(&references(&[&["Orders"]])),
    ))
    .expect("still resolves after the change");
    assert_eq!(plain, after, "changing `Orders` must not stale the review");
}

#[test]
fn postgres_quoted_orders_fingerprints_the_exact_case_table() {
    // The reverse: quoted `"Orders"` compares exactly, so changing the
    // quoted `Orders` table stales the review and changing `orders` does not.
    let quoted = complete(analyze(
        Some(&orders_tree()),
        SqlDialect::Postgres,
        Some(&quoted_references(&[&["Orders"]])),
    ))
    .expect("quoted `\"Orders\"` matches the exact-case table");
    let mut staled = orders_tree();
    stale_column(&mut staled, 1);
    let after = complete(analyze(
        Some(&staled),
        SqlDialect::Postgres,
        Some(&quoted_references(&[&["Orders"]])),
    ))
    .expect("still resolves after the change");
    assert_ne!(quoted, after, "changing `Orders` must stale the review");
    let mut staled = orders_tree();
    stale_column(&mut staled, 0);
    let after = complete(analyze(
        Some(&staled),
        SqlDialect::Postgres,
        Some(&quoted_references(&[&["Orders"]])),
    ))
    .expect("still resolves after the change");
    assert_eq!(quoted, after, "changing `orders` must not stale the review");
}

#[test]
fn postgres_quoted_parts_are_never_folded() {
    // `"PUBLIC".orders` is not `public.orders`: a quoted part compares
    // exactly, so it must not resolve — while the bare-spelled name does.
    let refs = SqlReferences {
        objects: vec![vec!["PUBLIC".to_string(), "orders".to_string()]],
        object_quoting: vec![vec![true, false]],
        columns: Vec::new(),
        partial: false,
    };
    let reason = unverifiable(analyze(
        Some(&postgres_tree()),
        SqlDialect::Postgres,
        Some(&refs),
    ));
    assert!(reason.contains("not found"), "{reason}");
    let analysis = analyze(
        Some(&postgres_tree()),
        SqlDialect::Postgres,
        Some(&quoted_references(&[&["public", "events"]])),
    );
    assert!(
        complete(analysis).is_some(),
        "the exact-case quote resolves"
    );
}

#[test]
fn snowflake_folds_unquoted_names_to_uppercase() {
    // Snowflake stores unquoted identifiers upper-cased: the unquoted SQL
    // `orders` IS the tree's `ORDERS`; a quoted `orders` is not.
    let tree = tree(vec![("analytics", vec![("public", vec!["ORDERS"])])]);
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Snowflake,
        Some(&references(&[&["orders"]])),
    );
    assert!(
        complete(analysis).is_some(),
        "unquoted `orders` folds to `ORDERS` and resolves"
    );
    let reason = unverifiable(analyze(
        Some(&tree),
        SqlDialect::Snowflake,
        Some(&quoted_references(&[&["orders"]])),
    ));
    assert!(reason.contains(r#"table "orders" not found"#), "{reason}");
}

#[test]
fn case_insensitive_dialects_fail_closed_on_case_collisions() {
    // SQLite/MySQL/DuckDB case behaviour is engine- or setting-dependent:
    // when more than one tree name matches case-insensitively the review
    // refuses — the spelling never picks among candidates (decision 3).
    let tree = tree_of_tables(vec![(
        "data",
        vec![(
            "main",
            vec![table("orders", &["a"]), table("Orders", &["b"])],
        )],
    )]);
    for dialect in [SqlDialect::Sqlite, SqlDialect::Mysql, SqlDialect::DuckDb] {
        for asked in ["orders", "Orders", "ORDERS"] {
            let reason = unverifiable(analyze(
                Some(&tree),
                dialect,
                Some(&references(&[&[asked]])),
            ));
            assert!(
                reason.contains(r#"ambiguous name ""#) && reason.contains(asked),
                "{dialect:?} {asked}: {reason}"
            );
        }
    }
}

#[test]
fn sqlite_single_table_resolves_in_any_case() {
    // One candidate: SQLite's case-insensitive matching resolves it however
    // the SQL spelled it.
    let tree = sqlite_tree();
    for asked in ["events", "EVENTS", "Events"] {
        let analysis = analyze(
            Some(&tree),
            SqlDialect::Sqlite,
            Some(&references(&[&[asked]])),
        );
        assert!(complete(analysis).is_some(), "{asked} must resolve");
    }
}

#[test]
fn clickhouse_matches_case_sensitively_regardless_of_quoting() {
    // ClickHouse never folds: only the exact-case tree name resolves, quoted
    // or not, and each exact-case name binds its own table's fingerprint.
    let tree = tree_of_tables(vec![(
        "analytics",
        vec![(
            "analytics",
            vec![table("orders", &["a"]), table("Orders", &["b"])],
        )],
    )]);
    let lower = complete(analyze(
        Some(&tree),
        SqlDialect::ClickHouse,
        Some(&references(&[&["analytics", "orders"]])),
    ))
    .expect("the exact-case bare name resolves");
    let upper = complete(analyze(
        Some(&tree),
        SqlDialect::ClickHouse,
        Some(&quoted_references(&[&["analytics", "Orders"]])),
    ))
    .expect("the exact-case quoted name resolves");
    assert_ne!(
        lower, upper,
        "each exact-case name resolved to its own table"
    );
    let reason = unverifiable(analyze(
        Some(&tree),
        SqlDialect::ClickHouse,
        Some(&references(&[&["analytics", "ORDERS"]])),
    ));
    assert!(
        reason.contains(r#"table "analytics.ORDERS" not found"#),
        "{reason}"
    );
}
