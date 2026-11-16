#[tokio::test]
async fn fixture_is_deterministic() {
    let dir_a = temp_demo_dir("det-a");
    let dir_b = temp_demo_dir("det-b");
    fixture::ensure(&dir_a, false).await.unwrap();
    fixture::ensure(&dir_b, false).await.unwrap();
    let dump_a = dump(&fixture::db_path(&dir_a)).await;
    let dump_b = dump(&fixture::db_path(&dir_b)).await;
    assert!(!dump_a.is_empty(), "the dump covers every table");
    assert_eq!(
        dump_a, dump_b,
        "two builds must produce identical table contents"
    );
    let _ = fs::remove_dir_all(&dir_a);
    let _ = fs::remove_dir_all(&dir_b);
}

#[tokio::test]
async fn fixture_has_the_traps() {
    let dir = temp_demo_dir("traps");
    fixture::ensure(&dir, false).await.unwrap();
    let db = fixture::db_path(&dir);

    let multi_contact = scalar(
        &db,
        "SELECT count(*) FROM (SELECT customer_id FROM customer_contacts \
         GROUP BY customer_id HAVING count(*) >= 2)",
    )
    .await;
    assert!(
        multi_contact >= 1,
        "some customer must have 2+ contact rows (duplicate-join trap)"
    );
    assert!(
        scalar(&db, "SELECT count(*) FROM customers WHERE email IS NULL").await >= 1,
        "some customer email must be NULL"
    );
    assert!(
        scalar(
            &db,
            "SELECT count(*) FROM orders WHERE amount_cents IS NULL"
        )
        .await
            >= 1,
        "some order amount must be NULL"
    );
    assert!(
        scalar(
            &db,
            "SELECT count(*) FROM orders WHERE order_date = '2025-12-31'"
        )
        .await
            >= 1,
        "orders must hit the 2025-12-31 boundary"
    );
    assert!(
        scalar(
            &db,
            "SELECT count(*) FROM orders WHERE order_date = '2026-01-01'"
        )
        .await
            >= 1,
        "orders must hit the 2026-01-01 boundary"
    );
    let stale_active = scalar(
        &db,
        "SELECT count(*) FROM customers c WHERE c.status = 'active' AND NOT EXISTS \
         (SELECT 1 FROM orders o WHERE o.customer_id = c.id AND o.order_date > '2025-10-03')",
    )
    .await;
    assert!(
        stale_active >= 1,
        "an active customer must not have ordered in the last 90 days of the data"
    );
    let recent_inactive = scalar(
        &db,
        "SELECT count(*) FROM customers c WHERE c.status <> 'active' AND EXISTS \
         (SELECT 1 FROM orders o WHERE o.customer_id = c.id AND o.order_date > '2025-10-03')",
    )
    .await;
    assert!(
        recent_inactive >= 1,
        "recency and status must disagree in the other direction too"
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM customers WHERE email IS NOT NULL \
             AND email NOT LIKE '%@example.invalid'"
        )
        .await,
        0,
        "emails must be obviously fake"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn fixture_row_and_size_bounds() {
    let dir = temp_demo_dir("bounds");
    fixture::ensure(&dir, false).await.unwrap();
    let db = fixture::db_path(&dir);
    let counts = [
        (
            "customers",
            scalar(&db, "SELECT count(*) FROM customers").await,
        ),
        ("orders", scalar(&db, "SELECT count(*) FROM orders").await),
        (
            "customer_contacts",
            scalar(&db, "SELECT count(*) FROM customer_contacts").await,
        ),
    ];
    for (table, count) in counts {
        assert!(count > 0, "{table} must be populated");
        assert!(count <= 1000, "{table} must stay within 1000 rows: {count}");
    }
    let size = fs::metadata(&db).unwrap().len();
    assert!(
        size <= 5 * 1024 * 1024,
        "the demo file must stay within 5 MiB: {size}"
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM saya_demo_meta \
             WHERE key = 'fixture_version' AND value = '1'"
        )
        .await,
        1,
        "the meta table records fixture_version = 1"
    );
    let _ = fs::remove_dir_all(&dir);
}
