/// 1. Using a candidate once leaves its status `Candidate` and its origin
/// `AssistantInferred`. The operation writes nothing to the store — it cannot
/// promote, and a user who uses one and walks away must find it unchanged.
#[tokio::test]
async fn use_candidate_once_leaves_status_and_origin_unchanged() {
    let root = temp_root("use_once_unchanged");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // A live `Pending` candidate the operation admits, seeded on the
    // `knowledge_items` table `use_candidate_once` now reads.
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
    assert_eq!(before.state, KnowledgeState::Pending);
    assert_eq!(before.source, ClaimOrigin::AssistantInferred);

    use_candidate_once(&store, &id).await.unwrap();

    let after = store
        .get_knowledge_item(id.as_str())
        .await
        .unwrap()
        .expect("item still present");
    assert_eq!(
        after.state,
        KnowledgeState::Pending,
        "using a candidate must not confirm it"
    );
    assert_eq!(
        after.source,
        ClaimOrigin::AssistantInferred,
        "using a candidate must not change its origin"
    );
    // Nothing durable moved: the operation touched no row, so the timestamp
    // the store updates on every write is the item's own.
    assert_eq!(after.updated_unix_ms, before.updated_unix_ms);

    let _ = fs::remove_dir_all(root);
}

/// 2. The admitted candidate becomes recallable within the scope, under
/// `recall = "confirmed"`, where it would otherwise be excluded.
///
/// This exercises the recall-admission behaviour: it seeds a `Pending` item and
/// admits that item's id directly via the request's `admit_candidate`. The
/// `use_candidate_once` op's own refuse-non-candidate behaviour is covered by
/// the review-op tests below; both now read `knowledge_items`.
#[tokio::test]
async fn use_candidate_once_admits_it_within_the_scope() {
    let root = temp_root("use_once_admits");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let (p, obj, id) = seed_one_pending_item(&store).await;

    let tree = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("amount", "numeric", false)]),
    )]);
    let terms: Vec<String> = vec!["orders".to_string()];
    // Without the admission, the candidate is excluded under Confirmed.
    let without = recall(
        &store,
        recall_admitting_one(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(tree.clone()))],
            &terms,
            None,
        ),
    )
    .await;
    assert!(
        without.contracts.is_empty(),
        "a candidate is not recallable under confirmed without admission"
    );

    // With the admission, the candidate's claim reaches the contract.
    let with = recall(
        &store,
        recall_admitting_one(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(tree))],
            &terms,
            Some(id.clone()),
        ),
    )
    .await;
    assert_eq!(
        with.contracts.len(),
        1,
        "the admitted candidate is selected"
    );
    assert_eq!(with.contracts[0].object, obj);
    let admitted: Vec<ClaimId> = with.contracts[0]
        .claims
        .iter()
        .map(|c| c.id.clone())
        .collect();
    assert!(
        admitted.contains(&id),
        "the named candidate is in the contract"
    );
    // The admitted claim is still `Candidate` in the contract recall returns.
    // The render layer marks a claim `[candidate — unconfirmed]` solely from
    // `status == Candidate`, so this is the property that keeps an admitted
    // candidate indistinguishable-from-nothing-special in the prompt: being
    // chosen for one turn confers no authority. If this read
    // `Confirmed`, the admission would have silently promoted it.
    let stored_admitted = with.contracts[0]
        .claims
        .iter()
        .find(|c| c.id == id)
        .expect("the admitted claim is in the contract");
    assert_eq!(
        stored_admitted.status,
        ClaimStatus::Candidate,
        "the admitted claim stays Candidate — it still renders unconfirmed"
    );

    let _ = fs::remove_dir_all(root);
}

/// 3. Outside the scope it is not recallable again. The admission is
/// request-scoped: a second recall, without the admission, excludes it — even
/// on the very next call against the same store.
#[tokio::test]
async fn use_candidate_once_does_not_persist_across_recalls() {
    let root = temp_root("use_once_not_persisted");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let (p, _obj, id) = seed_one_pending_item(&store).await;

    let tree = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("amount", "numeric", false)]),
    )]);
    let terms: Vec<String> = vec!["orders".to_string()];
    let schemas = &[(p.clone(), avail(tree))][..];
    let profiles = std::slice::from_ref(&p);

    // First recall admits it.
    let first = recall(
        &store,
        recall_admitting_one(profiles, schemas, &terms, Some(id)),
    )
    .await;
    assert_eq!(first.contracts.len(), 1);

    // A second recall, with no admission, excludes it again. The admission
    // died with the first request.
    let second = recall(
        &store,
        recall_admitting_one(profiles, schemas, &terms, None),
    )
    .await;
    assert!(
        second.contracts.is_empty(),
        "the admission is request-scoped: a later recall excludes the candidate again"
    );

    let _ = fs::remove_dir_all(root);
}

/// 4. A rejected / forgotten / confirmed claim is refused with a typed error.
/// The operation admits a live `Pending` candidate only; the rest are not
/// silently no-op'd. Under D-4 a "stale" fact is `Active` (state) reading
/// `Invalid` (computed), so it is refused as a non-candidate alongside an
/// explicit `Active` (confirmed) and a `Dismissed` (rejected/forgotten).
#[tokio::test]
async fn use_candidate_once_refuses_non_candidate_claims() {
    let root = temp_root("use_once_refuses");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");

    // Dismissed (rejected/forgotten): refused, unchanged.
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("rejected").unwrap(),
        KnowledgeState::Dismissed,
    )
    .await;
    let rejected_id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;
    let err = use_candidate_once(&store, &rejected_id).await.unwrap_err();
    assert_eq!(err, ContractOpError::NotACandidate, "dismissed is refused");
    assert_eq!(
        item_state(&store, &rejected_id).await,
        KnowledgeState::Dismissed,
        "the refusal changed nothing"
    );

    // Active (confirmed / the D-4 "stale" analog): refused, unchanged. An
    // Active item is already admissible by the mode, so use-once is a category
    // error, not a silent success.
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("active fact").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    let active_id = item_id_for(&store, &obj, &KnowledgeSlot::TableDescription).await;
    let err = use_candidate_once(&store, &active_id).await.unwrap_err();
    assert_eq!(err, ContractOpError::NotACandidate, "active is refused");

    let _ = fs::remove_dir_all(root);
}
