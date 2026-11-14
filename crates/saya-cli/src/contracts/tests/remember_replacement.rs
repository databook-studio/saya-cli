/// Single-valued slot replacement:
/// 1. Remember a single-valued grain, then remember a DIFFERENT grain on the same object;
///    assert the stored value is the SECOND one and the outcome is `Replaced` naming the first.
/// 2. A dismissed tombstone still reports `Duplicate`.
#[tokio::test]
async fn remember_single_slot_different_value_replaces_and_names_previous() {
    let root = temp_root("remember_replace_single_slot");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "rental");

    let first_grain = ClaimPayload::table_grain("one row per rental", None).unwrap();
    let second_grain = ClaimPayload::table_grain("one row per rental per day", None).unwrap();
    let unobserved_fp = crate::commands::unobserved_fingerprint();

    let outcome1 = remember(&store, &obj, &first_grain, unobserved_fp.clone())
        .await
        .unwrap();
    let id1 = match outcome1 {
        RememberOutcome::Stored { id } => id,
        other => panic!("first remember should be Stored, got {other:?}"),
    };

    let items1 = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(items1.len(), 1);
    assert_eq!(items1[0].value, first_grain);

    // 1. Remember a DIFFERENT grain on the same object:
    // must replace the stored value and return Replaced naming the previous value.
    let outcome2 = remember(&store, &obj, &second_grain, unobserved_fp.clone())
        .await
        .unwrap();
    match outcome2 {
        RememberOutcome::Replaced { id, previous } => {
            assert_eq!(id, id1);
            assert_eq!(previous, "one row per rental");
        }
        other => panic!("expected Replaced, got {other:?}"),
    }

    let items2 = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(items2.len(), 1);
    assert_eq!(
        items2[0].value, second_grain,
        "store must contain the second (replacement) value"
    );

    // 2. A dismissed tombstone still reports Duplicate.
    forget(&store, &id1, ForgetReason::Incorrect).await.unwrap();
    let third_grain = ClaimPayload::table_grain("one row per customer rental", None).unwrap();
    let outcome3 = remember(&store, &obj, &third_grain, unobserved_fp)
        .await
        .unwrap();
    match outcome3 {
        RememberOutcome::Duplicate { id, state } => {
            assert_eq!(id, id1);
            assert_eq!(state, KnowledgeState::Dismissed);
        }
        other => panic!("expected Duplicate with Dismissed state, got {other:?}"),
    }

    let _ = fs::remove_dir_all(root);
}
