/// 5. An Active (confirmed) item is refused, not a no-op. The operation's
/// contract is "admit an unconfirmed candidate"; a confirmed item is already
/// admissible by the mode, so using it once is a category error. Refusing
/// (rather than silently succeeding) keeps the caller from believing it did
/// something it did not — the same fail-closed posture as the other
/// non-candidate refusals.
#[tokio::test]
async fn use_candidate_once_refuses_a_confirmed_claim() {
    let root = temp_root("use_once_confirmed");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    let confirmed_id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;

    let err = use_candidate_once(&store, &confirmed_id).await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::NotACandidate,
        "a confirmed item is refused, not a silent no-op"
    );
    // And the item is untouched.
    assert_eq!(
        item_state(&store, &confirmed_id).await,
        KnowledgeState::Active
    );

    let _ = fs::remove_dir_all(root);
}

/// 6. Only the named claim is admitted; a sibling candidate on the same object
/// is not. The admission is per-claim, not per-object — the whole point of the
/// feature is to avoid the `include-candidates`-for-everything shape.
///
/// Seeds two `Pending` D-3 items on one object with distinct, valid bindings
/// (a `column_description` on `amount` and a `table_alias`), so validity does
/// not drop either — admission alone decides. See the note on
/// `use_candidate_once_admits_it_within_the_scope` for why `use_candidate_once`
/// itself is not called here.
#[tokio::test]
async fn use_candidate_once_admits_only_the_named_claim() {
    use saya_store::KnowledgeItemStore;
    let root = temp_root("use_once_only_named");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let (p, obj, admitted_id) = seed_one_pending_item(&store).await;

    // A sibling Pending item on the same object: a table alias (a `Table`
    // binding, valid against the orders table).
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("sibling").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let sibling_id = ClaimId::parse(
        &store
            .knowledge_for_object(&obj)
            .await
            .expect("knowledge items listed")
            .into_iter()
            .find(|i| i.slot == KnowledgeSlot::TableAlias)
            .expect("sibling item stored")
            .id,
    )
    .expect("ki id");
    assert_ne!(sibling_id, admitted_id);

    let tree = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("amount", "numeric", false)]),
    )]);
    let terms: Vec<String> = vec!["orders".to_string()];
    let outcome = recall(
        &store,
        recall_admitting_one(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(tree))],
            &terms,
            Some(admitted_id.clone()),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1, "the object is selected");
    let ids: Vec<ClaimId> = outcome.contracts[0]
        .claims
        .iter()
        .map(|c| c.id.clone())
        .collect();
    assert!(
        ids.contains(&admitted_id),
        "the named candidate is admitted"
    );
    assert!(
        !ids.contains(&sibling_id),
        "the sibling candidate is not admitted — admission is per-claim"
    );

    let _ = fs::remove_dir_all(root);
}

/// 7. Using a candidate writes nothing to the store. Using is not an
/// observation about the database; the operation touches no row, so the item's
/// state and write timestamp are its own afterwards. (`contract_evidence` is
/// gone, so "no evidence" is now structural — there is no table to write.)
#[tokio::test]
async fn use_candidate_once_creates_no_evidence() {
    let root = temp_root("use_once_no_evidence");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::column_description("amount", "how much").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(
        &store,
        &obj,
        &KnowledgeSlot::ColumnDescription {
            column: "amount".into(),
        },
    )
    .await;

    let before = store
        .get_knowledge_item(id.as_str())
        .await
        .unwrap()
        .expect("item present");

    use_candidate_once(&store, &id).await.unwrap();

    let after = store
        .get_knowledge_item(id.as_str())
        .await
        .unwrap()
        .expect("item still present");
    assert_eq!(after.state, KnowledgeState::Pending);
    assert_eq!(
        after.updated_unix_ms, before.updated_unix_ms,
        "using a candidate writes nothing; the timestamp is unchanged"
    );

    let _ = fs::remove_dir_all(root);
}
