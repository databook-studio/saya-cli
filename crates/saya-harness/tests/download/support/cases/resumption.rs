use super::*;

// --- test 2: killed mid-download, resume completes to the whole digest ------

/// The first attempt is killed mid-body (the server writes part of the
/// promised bytes and dies — a transport failure, not a short body). The
/// resume asks for exactly the range past the partial, completes, and the
/// final digest is the whole content's.
#[tokio::test]
async fn a_killed_download_resumes_and_completes_to_the_whole_digest() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("kill");
    let site = LocalSite::spawn(vec![(
        "/doc",
        Route {
            abort_after: Some(16),
            ..route(&content, 8)
        },
    )])
    .await;
    let net = transport(&site);

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
    .expect_err("the killed transfer is a transport failure");
    assert!(matches!(error, DownloadError::Transport(_)), "{error:?}");
    let partial = std::fs::read(root.join(PART)).expect("the partial exists");
    assert_eq!(partial, content[..16]);

    // The server recovers; the resume continues from the recorded offset.
    site.routes.lock().expect("routes poisoned")[0]
        .1
        .abort_after = None;
    let outcome = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect("the resume completes");
    assert_eq!(outcome.bytes, 40);
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(std::fs::read(root.join(DEST)).expect("dest"), content);
    assert!(!root.join(SIDECAR).exists());
    assert_eq!(
        site.ranges(),
        vec![None, Some(16)],
        "the resume asked for exactly the range past the partial"
    );
}

// --- test 3: a tampered partial is refused on digest, typed ------------------

/// The partial's bytes were tampered with between the kill and the resume:
/// the sidecar digest no longer matches what is on disk, so the resume is a
/// typed refusal — the evidence stays in place, nothing is restarted over.
#[tokio::test]
async fn a_tampered_partial_is_refused_on_digest() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("tamper");
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
    .expect_err("the first attempt is killed");

    let mut tampered = content[..16].to_vec();
    tampered[3] ^= 0xff;
    workspace
        .write(PART, &tampered)
        .expect("tamper the partial");

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
    .expect_err("the tampered partial is refused on digest");
    assert!(
        matches!(error, DownloadError::ResumeMismatch { .. }),
        "typed, not a shrug: {error:?}"
    );
    assert_eq!(
        std::fs::read(root.join(PART)).expect("the evidence stays"),
        tampered,
        "the partial is left in place"
    );
    assert!(!root.join(DEST).exists());
}
