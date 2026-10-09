/// Regression (spec §4.1): a plural prompt term selects the singular table it
/// names. `rentals` must match the object `rental`. Fails before the fix.
#[tokio::test]
async fn plural_prompt_term_selects_the_singular_table() {
    let root = temp_root("plural_selects_singular");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "rental");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("one row per rental").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[("rental", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["rentals".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        1,
        "plural term `rentals` must select the singular `rental` table, got {}",
        outcome.contracts.len()
    );
    assert_eq!(outcome.contracts[0].object, obj);

    let _ = fs::remove_dir_all(root);
}

/// Spec §4.2: the singular still selects — the fix must not regress the plain
/// case. Term `rental` selects the `rental` table.
#[tokio::test]
async fn singular_term_still_selects() {
    let root = temp_root("singular_still_selects");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "rental");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("one row per rental").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[("rental", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["rental".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    assert_eq!(outcome.contracts[0].object, obj);

    let _ = fs::remove_dir_all(root);
}

/// Spec §4.3: a non-plural word ending in `s` is not mangled into a wrong match.
/// `status`, `address`, `staff` each name a table; none must be singularized
/// (`status` not → `statu`, `address` not → `addres`, `staff` is `s`-free) and
/// each must still match its own name, and only its own.
#[tokio::test]
async fn irregular_s_words_are_not_mangled() {
    let root = temp_root("irregular_s_words");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    // Three objects whose names end in `s` or would be mis-singularized.
    let status = object_ref(&p, "status");
    let address = object_ref(&p, "address");
    let staff = object_ref(&p, "staff");
    for obj in [&status, &address, &staff] {
        put_item(
            &store,
            obj,
            ClaimPayload::table_description("a table").unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }
    let schema = schema_tree_for(&[
        ("status", table(&[("id", "bigint", false)])),
        ("address", table(&[("id", "bigint", false)])),
        ("staff", table(&[("id", "bigint", false)])),
    ]);

    // Each term selects exactly its own table — none is mis-singularized into
    // selecting another (e.g. `status` must not collapse to `statu` and so fail
    // to match, nor match `address`/`staff`).
    for (term, want) in [
        ("status", &status),
        ("address", &address),
        ("staff", &staff),
    ] {
        let outcome = recall(
            &store,
            recall_request(
                std::slice::from_ref(&p),
                &[(p.clone(), avail(schema.clone()))],
                &[term.to_string()],
                true,
                RecallBounds::defaults(),
            ),
        )
        .await;
        assert_eq!(
            outcome.contracts.len(),
            1,
            "term `{term}` should select exactly one object, got {}",
            outcome.contracts.len()
        );
        assert_eq!(
            outcome.contracts[0].object, *want,
            "term `{term}` selected the wrong object"
        );
    }

    let _ = fs::remove_dir_all(root);
}
