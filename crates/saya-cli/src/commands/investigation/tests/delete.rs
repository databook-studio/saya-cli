#[tokio::test]
async fn delete_checks_revision_then_removes_document_and_binding() {
    let root = temp_root("delete");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let id = saved_id(&repo, &runtime).await;

    // A stale --revision is refused with the current revision named.
    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Delete {
            id: id.clone(),
            revision: Some(5),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2, "err: {err}");
    assert!(err.contains("at revision 1"), "err: {err}");
    assert!(
        document_path(&root, &id).exists(),
        "refused delete changes nothing"
    );

    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Delete {
            id: id.clone(),
            revision: Some(1),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    assert!(!document_path(&root, &id).exists(), "document removed");
    assert!(!binding_path(&root, &id).exists(), "binding removed too");

    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Show { id: id.clone() },
        &runtime,
    )
    .await;
    assert_eq!(code, 2);
    assert!(
        err.contains(&format!("no investigation {id}")),
        "err: {err}"
    );
    let (_, out, _) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(out, "No saved investigations.\n");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn delete_refuses_a_document_it_cannot_verify() {
    let root = temp_root("delete-corrupt");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let id = saved_id(&repo, &runtime).await;
    fs::write(document_path(&root, &id), "not json at all").unwrap();
    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Delete {
            id: id.clone(),
            revision: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2, "a corrupt document is never deleted: {err}");
    assert!(err.contains("corrupt or invalid"), "err: {err}");
    assert!(document_path(&root, &id).exists(), "the file stays");
    let _ = fs::remove_dir_all(root);
}
