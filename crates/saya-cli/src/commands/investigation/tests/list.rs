#[tokio::test]
async fn list_empty_says_no_saved_investigations() {
    let root = temp_root("list-empty");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "err: {err}");
    assert_eq!(out, "No saved investigations.\n");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn list_reports_unreadable_documents_as_warnings() {
    let root = temp_root("list-warning");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    fs::create_dir_all(root.join("investigations")).unwrap();
    fs::write(
        root.join("investigations").join("bad-doc-1.json"),
        "not json at all",
    )
    .unwrap();
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "a bad file warns, never fails the page: {err}");
    assert!(out.contains("warning: bad-doc-1"), "out: {out}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn list_clamps_limit_and_honours_offset() {
    let root = temp_root("list-page");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    for (first, second) in [("First", "SELECT 1"), ("Second", "SELECT 2")] {
        let (code, out, err) = run(
            &repo,
            InvestigationCommand::Save {
                name: first.into(),
                description: None,
                sql: Some(second.into()),
                file: None,
                connection: None,
                param_specs: Vec::new(),
            },
            &runtime,
        )
        .await;
        assert_eq!(code, 0, "out: {out} err: {err}");
    }
    // limit 0 and limit 500 both clamp into the repository's 1..=50 page.
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::List {
            limit: Some(0),
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "limit 0 clamps, never errors: {err}");
    assert!(
        out.contains("more: --offset 1"),
        "ordinary pagination names the next offset: {out}"
    );
    let summaries = out
        .lines()
        .filter(|line| !line.starts_with("more:"))
        .count();
    assert_eq!(summaries, 1, "limit 0 clamps to one summary: {out}");
    let (_, out, _) = run(
        &repo,
        InvestigationCommand::List {
            limit: Some(500),
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(out.lines().count(), 2, "both fit inside the 50-page: {out}");
    assert!(
        !out.contains("(capped at 500)") && !out.contains("more:"),
        "nothing beyond this page: {out}"
    );
    // offset skips past the first id in id order.
    let (_, all, _) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
    )
    .await;
    let (_, shifted, _) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: Some(1),
        },
        &runtime,
    )
    .await;
    let first_of = |text: &str| text.lines().next().unwrap().to_string();
    assert_eq!(shifted.trim(), all.lines().nth(1).unwrap().trim());
    assert_ne!(first_of(&shifted), first_of(&all));
    let _ = fs::remove_dir_all(root);
}
