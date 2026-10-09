use super::*;

// --- test 7: the destination is contained; a sentinel outside is unchanged --

/// A `..` escape is refused by the same path discipline as
/// `workspace_write`; a sentinel outside the workspace is unchanged and no
/// request is ever made.
#[tokio::test]
async fn the_destination_is_contained_and_a_sentinel_outside_is_unchanged() {
    let (root, workspace) = workspace("contain");
    let sentinel = root.parent().unwrap().join("sentinel.txt");
    std::fs::write(&sentinel, b"keep me").expect("sentinel outside");
    let site = LocalSite::spawn(vec![("/doc", route(b"payload", 4))]).await;
    let net = transport(&site);

    for destination in ["../sentinel.txt", "out/../../escape.txt", "/etc/passwd"] {
        let error = http_download(
            &policy(),
            &net,
            &workspace,
            destination,
            &url("/doc"),
            DownloadLimits::default(),
            &DownloadBudget::default(),
        )
        .await
        .expect_err("the escape is refused");
        assert!(matches!(error, DownloadError::Workspace(_)), "{error:?}");
    }
    assert_eq!(
        std::fs::read(&sentinel).expect("sentinel"),
        b"keep me",
        "the sentinel outside the workspace is unchanged"
    );
    assert!(
        std::fs::read_dir(&root)
            .expect("workspace")
            .next()
            .is_none(),
        "nothing was written into the workspace"
    );
    assert!(site.served().is_empty(), "no request was ever made");
}
