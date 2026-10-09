#[tokio::test]
async fn save_slash_and_clap_agree_on_the_command_and_the_bytes() {
    let root = temp_root("save_parity");
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;

    let (_clap, slash_code, slash_out, slash_err) = save_document(&runtime, &store).await;
    assert_eq!(slash_code, 0, "save stderr: {slash_err}");
    let clap_saved = run_headless(_clap.clone(), &runtime, &store).await;
    assert_eq!(clap_saved.0, 0, "clap save stderr: {}", clap_saved.2);

    assert_eq!(
        mask_save_volatile(&slash_out),
        mask_save_volatile(&clap_saved.1),
        "slash save diverged from clap save"
    );
    assert_eq!(slash_err, clap_saved.2);

    // The document is really there: list names it.
    let list = run_headless(
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
        &store,
    )
    .await;
    assert!(list.1.contains("Total orders"), "list out: {}", list.1);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. list and show: byte for byte.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn list_and_show_slash_and_clap_agree_byte_for_byte() {
    let root = temp_root("list_show_parity");
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, code, out, err) = save_document(&runtime, &store).await;
    assert_eq!(code, 0, "seed save stderr: {err}");
    let id = out.lines().next().unwrap().to_string();

    let clap_list = run_headless(clap_investigation(&["list"]), &runtime, &store).await;
    let slash_list =
        run_headless(slash_investigation("/investigation list"), &runtime, &store).await;
    assert_eq!(
        slash_list.1, clap_list.1,
        "slash list diverged from clap list"
    );
    assert_eq!(slash_list.2, clap_list.2);

    let clap_show = run_headless(clap_investigation(&["show", &id]), &runtime, &store).await;
    let slash_show = run_headless(
        slash_investigation(&format!("/investigation show {id}")),
        &runtime,
        &store,
    )
    .await;
    assert_eq!(slash_show.0, 0, "show stderr: {}", slash_show.2);
    assert_eq!(
        slash_show.1, clap_show.1,
        "slash show diverged from clap show"
    );
    assert_eq!(slash_show.2, clap_show.2);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 3. delete: byte for byte, and the document is really gone.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn delete_slash_and_clap_agree_byte_for_byte() {
    let root = temp_root("delete_parity");
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, code, out, err) = save_document(&runtime, &store).await;
    assert_eq!(code, 0, "seed save stderr: {err}");
    let id = out.lines().next().unwrap().to_string();

    let slash_delete = run_headless(
        slash_investigation(&format!("/investigation delete {id}")),
        &runtime,
        &store,
    )
    .await;
    assert_eq!(slash_delete.0, 0, "delete stderr: {}", slash_delete.2);

    // The clap delete of the (now gone) id must fail exactly as a slash
    // delete of it would — same class, same bytes.
    let clap_delete = run_headless(clap_investigation(&["delete", &id]), &runtime, &store).await;
    let slash_delete_again = run_headless(
        slash_investigation(&format!("/investigation delete {id}")),
        &runtime,
        &store,
    )
    .await;
    assert_ne!(clap_delete.0, 0, "deleting a missing id must fail");
    assert_eq!(
        slash_delete_again.1, clap_delete.1,
        "slash delete diverged from clap delete"
    );
    assert_eq!(slash_delete_again.2, clap_delete.2);

    // And the deleted document no longer lists.
    let list = run_headless(
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
        &store,
    )
    .await;
    assert!(
        list.1.contains("No saved investigations."),
        "the deleted document must be gone: {}",
        list.1
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 4. `/investigations` is `list`, producing the same bytes.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn investigations_alias_output_equals_investigation_list() {
    let root = temp_root("alias_parity");
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, code, out, err) = save_document(&runtime, &store).await;
    assert_eq!(code, 0, "seed save stderr: {err}");
    let id = out.lines().next().unwrap().to_string();

    let alias = run_headless(slash_investigation("/investigations"), &runtime, &store).await;
    let list = run_headless(slash_investigation("/investigation list"), &runtime, &store).await;
    assert_eq!(alias.0, 0, "alias stderr: {}", alias.2);
    assert_eq!(
        alias.1, list.1,
        "/investigations diverged from /investigation list"
    );
    assert!(
        alias.1.contains(&id),
        "the seeded id is listed: {}",
        alias.1
    );

    let _ = fs::remove_dir_all(root);
}
