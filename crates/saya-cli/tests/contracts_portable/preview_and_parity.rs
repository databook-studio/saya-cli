#[tokio::test]
async fn context_import_preview_writes_nothing() {
    let root = temp_root("preview");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema(&["orders"]))
        .await
        .unwrap();

    let items = vec![context_item(
        "analytics",
        "public",
        "orders",
        &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
    )];
    let path = root.join("context.json");
    write_context_file(&path, &context_document(items));

    let (code, out, err) = run(
        ContractsCommand::Import {
            path,
            profile: None,
            preview: true,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "preview failed: {out}{err}");
    assert!(out.contains("would import 1"), "the preview summary: {out}");
    assert!(
        out.contains("Preview only: nothing was written."),
        "the preview posture: {out}"
    );
    assert!(
        stored_items(&store, &runtime, None, "analytics.public.orders")
            .await
            .is_empty(),
        "a preview writes nothing"
    );

    // The dbt preview behaves the same.
    let manifest = root.join("manifest.json");
    fs::write(
        &manifest,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/contracts/dbt/fixtures/v12_manifest.json"
        )),
    )
    .unwrap();
    let (code, out, err) = run(
        ContractsCommand::ImportDbt {
            manifest,
            profile: None,
            select: vec![],
            preview: true,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "dbt preview failed: {out}{err}");
    assert!(
        out.contains("would import"),
        "the dbt preview summary: {out}"
    );
    assert!(
        out.contains("Preview only: nothing was written."),
        "the dbt preview posture: {out}"
    );
    assert!(
        stored_items(&store, &runtime, None, "analytics.core.public_orders")
            .await
            .is_empty(),
        "a dbt preview writes nothing"
    );

    drop(store);
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// slash parity: the same enum, not a second parser
// ---------------------------------------------------------------------------

#[test]
fn slash_export_import_import_dbt_parse_matches_headless() {
    for (line, expected) in [
        (
            "/contracts export /tmp/context.json",
            ContractsCommand::Export {
                path: "/tmp/context.json".into(),
                profile: None,
                overwrite: false,
            },
        ),
        (
            "/contracts export /tmp/context.json --overwrite",
            ContractsCommand::Export {
                path: "/tmp/context.json".into(),
                profile: None,
                overwrite: true,
            },
        ),
        (
            "/contracts import /tmp/context.json --preview",
            ContractsCommand::Import {
                path: "/tmp/context.json".into(),
                profile: None,
                preview: true,
            },
        ),
        (
            "/contracts import-dbt /tmp/manifest.json",
            ContractsCommand::ImportDbt {
                manifest: "/tmp/manifest.json".into(),
                profile: None,
                select: vec![],
                preview: false,
            },
        ),
        (
            "/contracts import-dbt /tmp/manifest.json --select model.jaffle.* --preview",
            ContractsCommand::ImportDbt {
                manifest: "/tmp/manifest.json".into(),
                profile: None,
                select: vec!["model.jaffle.*".into()],
                preview: true,
            },
        ),
    ] {
        let parsed = match parse_slash_command(line) {
            Ok(Some(SlashCommand::Contracts(command))) => command,
            other => panic!("{line:?} must parse to a contracts command, got {other:?}"),
        };
        assert_eq!(parsed, expected, "{line:?} must equal the headless command");
    }

    // Missing paths and unknown flags are usage errors, never silent ops.
    for line in [
        "/contracts export",
        "/contracts import --preview",
        "/contracts import-dbt",
        "/contracts export /tmp/a.json --bogus",
        "/contracts import-dbt /tmp/m.json --select",
    ] {
        assert!(
            parse_slash_command(line).is_err(),
            "{line:?} must be a usage error"
        );
    }
}
