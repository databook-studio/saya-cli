#[tokio::test]
async fn json_and_ndjson_carry_same_claim_ids_as_text() {
    let root = temp_root("formats");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    // The text `remember` line confirms the fact in words without raw id.
    let (code, text_out, err) = run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        text_out.contains("remembered alias customers for analytics.public.orders"),
        "text form names the fact and object: {text_out}"
    );
    assert!(
        !text_out.contains("ki-"),
        "text form drops raw id: {text_out}"
    );

    let identity = identity_for(&runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    let stored_id = items[0].id.as_str();

    // The duplicate event (same value remembered again) in JSON and NDJSON
    // carries `contract_remembered`.
    for format in [RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, out, err) = run(remember.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "{format:?} stderr: {err}");
        let events = json_events(&out);
        let remembered_evt = events
            .iter()
            .find(|v| v["event"] == "contract_remembered")
            .expect("a contract_remembered event was emitted");
        assert_eq!(remembered_evt["action"], "duplicate");
        assert_eq!(
            remembered_evt["status"], "confirmed",
            "duplicate of a confirmed claim carries its status"
        );
        assert_eq!(remembered_evt["object"], "analytics.public.orders");
        assert_eq!(remembered_evt["kind"], "alias");
        assert_eq!(remembered_evt["value"], "customers");
        // The id left the text confirmation but not the machine surface: a
        // script that remembers then forgets still has a handle.
        assert_eq!(
            remembered_evt["claim_id"], stored_id,
            "{format:?} remember id disagrees with stored id"
        );
    }

    // …and the same command rendered as text names the fact instead of the id.
    let (code, remember_text, err) =
        run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        !remember_text.contains(stored_id),
        "text confirmation must not print the raw id: {remember_text}"
    );

    // `show` carries the claim id in every structured format (JSON/NDJSON),
    // which matches the store's id.
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    for format in [RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, out, err) = run(show.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "{format:?} stderr: {err}");
        let events = json_events(&out);
        let shown = events
            .iter()
            .find(|v| v["event"] == "contract_show")
            .expect("a contract_show event was emitted");
        let claim_id = shown["contract"]["claims"][0]["claim_id"]
            .as_str()
            .expect("claim id present");
        assert_eq!(
            claim_id, stored_id,
            "{format:?} show id disagrees with stored id"
        );
    }

    // The text `show` line contains the value, so the round-trip is real.
    let (code, show_text, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(show_text.contains("customers"), "text show: {show_text}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 11. Phase 3d: `saya contracts queue` lists candidates, and confirming one
//     from the queue makes it recallable while rejecting it never does. The
//     confirm reaches the existing `confirm` op through `decide` (the survivor
//     of the `review` retirement) — no new write tool.
// ---------------------------------------------------------------------------
