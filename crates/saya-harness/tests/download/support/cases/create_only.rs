use super::*;

// --- destination intent: the per-call precondition ---------------------------

/// A create-only download over a destination that already exists is refused
/// before anything streams: no request is made, no part and no sidecar
/// appear, and the existing destination is untouched. The publication guard
/// would refuse the same call after a full body had streamed — the
/// pre-flight refuses it before the first byte is spent.
#[tokio::test]
async fn a_create_only_download_over_an_existing_destination_refuses_before_anything_streams() {
    let (root, workspace) = workspace("preexisting");
    std::fs::create_dir_all(root.join("downloads")).unwrap();
    std::fs::write(root.join(DEST), b"old destination").unwrap();
    let content = b"fresh body".to_vec();
    let site = LocalSite::spawn(vec![("/doc", route(&content, 4))]).await;
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
    .expect_err("create-only over an existing destination refuses");
    assert!(
        matches!(
            error,
            DownloadError::Workspace(HarnessError::AlreadyExists { ref path }) if path == DEST
        ),
        "typed refusal: {error:?}"
    );
    assert_eq!(
        std::fs::read(root.join(DEST)).unwrap(),
        b"old destination",
        "the existing destination is untouched"
    );
    assert!(
        site.served().is_empty(),
        "refused before any network I/O: {:?}",
        site.served()
    );
    let downloads: Vec<String> = std::fs::read_dir(root.join("downloads"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        downloads,
        vec!["doc.bin".to_owned()],
        "no part and no sidecar were created: {downloads:?}"
    );
}

/// A create-only retry over a destination that appeared while a valid
/// partial sat interrupted: the pre-flight refuses before any network I/O,
/// and the refused attempt leaves the destination, the partial, and its
/// sidecar exactly as they were — for an explicit retry that carries the
/// destination's digest.
#[tokio::test]
async fn a_create_only_retry_over_a_destination_that_appeared_refuses_and_keeps_the_partial() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("appeared");
    let site = LocalSite::spawn(vec![(
        "/doc",
        Route {
            abort_after: Some(16),
            ..route(&content, 8)
        },
    )])
    .await;
    let net = transport(&site);

    // Attempt 1: killed mid-body — a partial and its sidecar remain, and
    // the destination is still absent.
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
    assert_eq!(std::fs::read(root.join(PART)).unwrap(), content[..16]);
    assert!(!root.join(DEST).exists());

    // The destination appears between attempts; the retry carries no
    // digest, so it can only create — and something is already there.
    site.routes.lock().expect("routes poisoned")[0]
        .1
        .abort_after = None;
    std::fs::write(root.join(DEST), b"appeared meanwhile").unwrap();

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
    .expect_err("create-only over the appeared destination refuses");
    assert!(
        matches!(
            error,
            DownloadError::Workspace(HarnessError::AlreadyExists { ref path }) if path == DEST
        ),
        "typed refusal: {error:?}"
    );
    assert_eq!(
        site.served(),
        vec!["/doc".to_owned()],
        "the retry made no request of its own"
    );
    assert_eq!(
        std::fs::read(root.join(DEST)).unwrap(),
        b"appeared meanwhile",
        "the appeared destination is untouched"
    );
    assert_eq!(
        std::fs::read(root.join(PART)).unwrap(),
        content[..16],
        "the valid partial stays for an explicit retry"
    );
    let sidecar = sidecar_json(&root);
    assert_eq!(sidecar["len"], 16);
    assert_eq!(sidecar["sha256"], digest(&content[..16]));
}
