use super::*;

/// The caller's destination intent rides every attempt: the destination is
/// replaced between a killed attempt and its resume, so the resume carrying
/// the OLD digest streams to completion and is refused at publication
/// (`ContentChanged` — the stale digest no longer matches the destination's
/// current bytes), leaving the changed destination and the partial both in
/// place; the retry with the destination's real digest re-fetches the
/// unaccounted tail, replaces the destination, and clears the sidecar.
#[tokio::test]
async fn a_destination_changed_between_attempts_refuses_the_stale_digest_then_the_retry_replaces() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("stale-digest");
    std::fs::create_dir_all(root.join("downloads")).unwrap();
    std::fs::write(root.join(DEST), b"old destination").unwrap();
    let old_digest = digest(b"old destination");
    let site = LocalSite::spawn(vec![(
        "/doc",
        Route {
            abort_after: Some(16),
            ..route(&content, 8)
        },
    )])
    .await;
    let net = transport(&site);
    let policy = policy();
    let target = url("/doc");
    let budget = DownloadBudget::default();
    let token = CancellationToken::new();

    // Attempt 1: the caller holds the destination's current digest and the
    // transfer is killed mid-body — a partial and its sidecar remain.
    let error = http_download_cancellable(DownloadCall {
        policy: &policy,
        transport: &net,
        workspace: &workspace,
        destination: DEST,
        expected_destination_sha256: Some(&old_digest),
        url: &target,
        limits: DownloadLimits::default(),
        budget: &budget,
        cancellation: &token,
    })
    .await
    .expect_err("the killed transfer is a transport failure");
    assert!(matches!(error, DownloadError::Transport(_)), "{error:?}");
    assert_eq!(std::fs::read(root.join(PART)).unwrap(), content[..16]);
    assert_eq!(
        std::fs::read(root.join(DEST)).unwrap(),
        b"old destination",
        "the unchanged destination survives the killed attempt"
    );

    // Between attempts the destination is replaced by something else.
    site.routes.lock().expect("routes poisoned")[0]
        .1
        .abort_after = None;
    std::fs::write(root.join(DEST), b"changed elsewhere").unwrap();
    let real_digest = digest(b"changed elsewhere");

    // Attempt 2: the stale digest streams to completion and is refused at
    // publication — the changed destination and the partial both stay.
    let error = http_download_cancellable(DownloadCall {
        policy: &policy,
        transport: &net,
        workspace: &workspace,
        destination: DEST,
        expected_destination_sha256: Some(&old_digest),
        url: &target,
        limits: DownloadLimits::default(),
        budget: &budget,
        cancellation: &token,
    })
    .await
    .expect_err("the stale digest refuses at publication");
    assert!(
        matches!(
            error,
            DownloadError::Workspace(HarnessError::ContentChanged {
                ref expected,
                ref current,
                ..
            }) if expected == &old_digest && current == &real_digest
        ),
        "the refusal names both digests: {error:?}"
    );
    assert_eq!(
        std::fs::read(root.join(DEST)).unwrap(),
        b"changed elsewhere",
        "the changed destination is untouched by the refused publication"
    );
    assert_eq!(
        std::fs::read(root.join(PART)).unwrap(),
        content,
        "the completed partial stays for the explicit retry"
    );
    assert!(root.join(SIDECAR).exists(), "the sidecar stays too");

    // Attempt 3: the destination's real digest replaces it — the recorded
    // prefix is kept, the unaccounted tail re-fetched, the part promoted,
    // and the sidecar cleared by the success alone.
    let outcome = http_download_cancellable(DownloadCall {
        policy: &policy,
        transport: &net,
        workspace: &workspace,
        destination: DEST,
        expected_destination_sha256: Some(&real_digest),
        url: &target,
        limits: DownloadLimits::default(),
        budget: &budget,
        cancellation: &token,
    })
    .await
    .expect("the retry with the real digest replaces");
    assert_eq!(outcome.bytes, 40);
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(std::fs::read(root.join(DEST)).unwrap(), content);
    assert!(!root.join(PART).exists() && !root.join(SIDECAR).exists());
    assert_eq!(
        site.ranges(),
        vec![None, Some(16), Some(16)],
        "each resume asked from the recorded offset"
    );
}

/// An explicit destination digest replaces a matching destination in one
/// fresh call: the publication guard hashes the destination's current
/// bytes, the rename lands, and the sidecar this attempt created is cleared
/// by the success alone.
#[tokio::test]
async fn an_explicit_matching_digest_replaces_the_destination_and_clears_the_sidecar() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("replace");
    std::fs::create_dir_all(root.join("downloads")).unwrap();
    std::fs::write(root.join(DEST), b"old destination").unwrap();
    let observed = digest(b"old destination");
    let site = LocalSite::spawn(vec![("/doc", route(&content, 8))]).await;
    let net = transport(&site);
    let policy = policy();
    let outcome = http_download_cancellable(DownloadCall {
        policy: &policy,
        transport: &net,
        workspace: &workspace,
        destination: DEST,
        expected_destination_sha256: Some(&observed),
        url: &url("/doc"),
        limits: DownloadLimits::default(),
        budget: &DownloadBudget::default(),
        cancellation: &CancellationToken::new(),
    })
    .await
    .expect("the matching digest replaces the destination");
    assert_eq!(outcome.bytes, 40);
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(std::fs::read(root.join(DEST)).unwrap(), content);
    assert!(!root.join(SIDECAR).exists(), "the sidecar is cleared");
    assert!(!root.join(PART).exists());
}

/// A digest grants no authority to create: a call carrying a destination
/// digest over an absent destination streams to completion and is refused
/// at publication (`NotFound`), with the valid partial and its sidecar left
/// in place for a create-only retry — never a fresh file.
#[tokio::test]
async fn a_digest_over_a_missing_destination_refuses_and_keeps_the_partial() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("digest-missing");
    std::fs::create_dir_all(root.join("downloads")).unwrap();
    let site = LocalSite::spawn(vec![("/doc", route(&content, 8))]).await;
    let net = transport(&site);
    let policy = policy();
    let observed = digest(&content);
    let error = http_download_cancellable(DownloadCall {
        policy: &policy,
        transport: &net,
        workspace: &workspace,
        destination: DEST,
        expected_destination_sha256: Some(&observed),
        url: &url("/doc"),
        limits: DownloadLimits::default(),
        budget: &DownloadBudget::default(),
        cancellation: &CancellationToken::new(),
    })
    .await
    .expect_err("a digest never authorizes creation");
    assert!(
        matches!(
            error,
            DownloadError::Workspace(HarnessError::NotFound { ref path }) if path == DEST
        ),
        "typed refusal: {error:?}"
    );
    assert!(!root.join(DEST).exists(), "nothing was created");
    assert_eq!(
        std::fs::read(root.join(PART)).unwrap(),
        content,
        "the completed partial stays for an explicit retry"
    );
    assert!(root.join(SIDECAR).exists());
}
