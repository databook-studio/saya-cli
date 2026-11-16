#[tokio::test]
async fn export_writes_active_claims_only_and_counts_by_kind() {
    let root = temp_root("export_active");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;

    remember_active(&store, &runtime, ClaimKindArg::Alias, "customers", None).await;
    remember_active(
        &store,
        &runtime,
        ClaimKindArg::Description,
        "Orders from the ERP.",
        None,
    )
    .await;
    remember_active(
        &store,
        &runtime,
        ClaimKindArg::TimeColumn,
        "return_date",
        None,
    )
    .await;
    // A pending and a dismissed row must never ride along: the file carries
    // Active claims only.
    seed_other_state(&store, &runtime, KnowledgeState::Pending).await;
    seed_other_state(&store, &runtime, KnowledgeState::Dismissed).await;

    let path = root.join("context.json");
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: path.clone(),
            profile: None,
            overwrite: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "export failed: {out}{err}");
    assert!(out.contains("exported 3 claims"), "summary counts: {out}");
    assert!(out.contains("table_alias 1"), "per-kind count: {out}");
    assert!(out.contains("table_description 1"), "per-kind count: {out}");
    assert!(
        out.contains("default_time_column 1"),
        "per-kind count: {out}"
    );
    assert!(
        out.contains("Review the file before sharing."),
        "the sharing caution: {out}"
    );
    assert!(
        !out.contains("ghost-alias"),
        "non-active claims excluded: {out}"
    );

    // The file is a valid document whose items are exactly the Active claims,
    // with the opaque identity stripped.
    let document = ContextDocumentV1::from_json_bytes(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(document.items.len(), 3);
    let identity = identity_for(&runtime, "local");
    assert!(
        !fs::read_to_string(&path).unwrap().contains(&identity),
        "the opaque identity must never reach the file"
    );
    assert!(
        document
            .items
            .iter()
            .all(|item| item.object.catalog.as_deref() == Some("analytics")
                && item.object.schema.as_deref() == Some("public")),
        "objects carry their logical names: {:?}",
        document.items
    );

    // A second export onto the existing file is refused without --overwrite.
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: path.clone(),
            profile: None,
            overwrite: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        code, 0,
        "existing file refused without --overwrite: {out}{err}"
    );
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: path.clone(),
            profile: None,
            overwrite: true,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "--overwrite replaces: {out}{err}");
    let replaced = ContextDocumentV1::from_json_bytes(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(replaced.items.len(), 3);

    drop(store);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn export_refuses_symlink_and_directory_targets() {
    let root = temp_root("export_targets");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;
    remember_active(&store, &runtime, ClaimKindArg::Alias, "customers", None).await;

    let dir = root.join("adir");
    fs::create_dir_all(&dir).unwrap();
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: dir.clone(),
            profile: None,
            overwrite: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(code, 0, "a directory target is refused: {out}{err}");

    // A symlink is refused even when --overwrite is passed: the atomic rename
    // would replace the link, not the file it points at, so the check is not
    // optional.
    let target = root.join("real.json");
    fs::write(&target, b"{}").unwrap();
    let link = root.join("link.json");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(not(unix))]
    fs::copy(&target, &link).unwrap();
    let (code, out, err) = run(
        ContractsCommand::Export {
            path: link.clone(),
            profile: None,
            overwrite: true,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    if cfg!(unix) {
        assert_ne!(code, 0, "a symlink target is refused: {out}{err}");
        assert!(
            out.contains("symbolic link") || err.contains("symbolic link"),
            "the refusal names the reason: {out}{err}"
        );
        assert_eq!(
            fs::read(&target).unwrap(),
            b"{}",
            "the link target untouched"
        );
    }

    drop(store);
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// import: pending-not-authority, mapping, conflicts, idempotency, atomicity
// ---------------------------------------------------------------------------
