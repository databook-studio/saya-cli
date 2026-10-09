use super::*;

/// Windows preserves destination creation metadata across `rename` in some
/// filesystems, and a resumed download therefore exercises the handle
/// identity checks for an appended part and its promotion. Publication is
/// create-only absent an explicit destination digest, so a pre-existing
/// destination refuses the promotion: the old content, the completed part,
/// and its sidecar all stay in place for an explicit retry — which carries
/// the destination's observed digest and replaces it.
#[cfg(windows)]
#[tokio::test]
async fn windows_resume_refuses_an_existing_destination_and_keeps_the_completed_part() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("windows-identity");
    std::fs::create_dir_all(root.join("downloads")).unwrap();
    std::fs::write(root.join(DEST), b"old destination").unwrap();
    let site = LocalSite::spawn(vec![(
        "/doc",
        Route {
            abort_after: Some(16),
            ..route(&content, 8)
        },
    )])
    .await;
    let net = transport(&site);

    http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect_err("the first download leaves a part to resume");
    assert_eq!(std::fs::read(root.join(PART)).unwrap(), content[..16]);

    site.routes.lock().expect("routes poisoned")[0]
        .1
        .abort_after = None;
    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect_err("the promotion refuses an existing destination absent a digest");
    assert!(
        matches!(
            error,
            DownloadError::Workspace(HarnessError::AlreadyExists { .. })
        ),
        "create-only refusal, typed: {error:?}"
    );
    assert_eq!(
        std::fs::read(root.join(DEST)).unwrap(),
        b"old destination",
        "the existing destination is untouched"
    );
    assert_eq!(
        std::fs::read(root.join(PART)).unwrap(),
        content,
        "the completed part stays for an explicit retry"
    );
    assert!(root.join(SIDECAR).exists(), "the sidecar stays too");

    // The explicit retry: the destination's observed digest replaces it.
    // The recorded prefix is kept, the unaccounted tail re-fetched, and the
    // completed part promoted over the destination at last.
    let observed = digest(b"old destination");
    let policy = policy();
    let target = url("/doc");
    let outcome = http_download_cancellable(DownloadCall {
        policy: &policy,
        transport: &net,
        workspace: &workspace,
        destination: DEST,
        expected_destination_sha256: Some(&observed),
        url: &target,
        limits: DownloadLimits::default(),
        budget: &DownloadBudget::default(),
        cancellation: &CancellationToken::new(),
    })
    .await
    .expect("the explicit replacement with the observed digest succeeds");
    assert_eq!(outcome.bytes, 40);
    assert_eq!(std::fs::read(root.join(DEST)).unwrap(), content);
    assert!(!root.join(SIDECAR).exists(), "the sidecar is cleared");
    assert!(!root.join(PART).exists());
}
