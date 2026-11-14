#[tokio::test]
async fn show_refuses_missing_malformed_newer_and_corrupt_ids() {
    let root = temp_root("show-refusals");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let id = saved_id(&repo, &runtime).await;

    // Malformed id: same refusal shape as a missing one.
    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Show {
            id: "NOT AN ID".into(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2);
    assert!(err.contains("no investigation"), "err: {err}");

    // A document written by a newer saya is refused, never downgraded.
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    fs::write(
        document_path(&root, &id),
        document.replace("\"version\": 1", "\"version\": 2"),
    )
    .unwrap();
    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Show { id: id.clone() },
        &runtime,
    )
    .await;
    assert_eq!(code, 2);
    assert!(err.contains("made by a newer saya"), "err: {err}");

    // A corrupt document is reported, never repaired or silently shown.
    fs::write(document_path(&root, &id), "not json at all").unwrap();
    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Show { id: id.clone() },
        &runtime,
    )
    .await;
    assert_eq!(code, 2);
    assert!(err.contains("corrupt or invalid"), "err: {err}");

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn show_reports_a_missing_binding_honestly() {
    let root = temp_root("show-binding");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let id = saved_id(&repo, &runtime).await;
    fs::remove_file(binding_path(&root, &id)).unwrap();
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Show { id: id.clone() },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "err: {err}");
    assert!(out.contains("local binding: none"), "out: {out}");
    let _ = fs::remove_dir_all(root);
}
