use super::*;

// --- test 4: a redirect to a refused destination is refused at that hop -----

/// The first hop's 302 points at the metadata endpoint: the hop is judged
/// and refused, the 302's body is never read, the refused target is never
/// connected to, and nothing is written.
#[tokio::test]
async fn a_redirect_to_a_refused_destination_is_refused_at_that_hop() {
    let (root, workspace) = workspace("redirect");
    let site = LocalSite::spawn(vec![(
        "/start",
        redirect("https://169.254.169.254/latest/meta-data/"),
    )])
    .await;
    let net = transport(&site);
    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/start"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect_err("the redirect target is refused");
    assert!(
        matches!(
            error,
            DownloadError::Refused(FetchRefusal::DisallowedAddress { ref host })
                if host == "169.254.169.254"
        ),
        "typed refusal at the refused hop: {error:?}"
    );
    assert_eq!(site.served(), vec!["/start".to_owned()]);
    assert!(
        !root.join(DEST).exists() && !root.join(PART).exists() && !root.join(SIDECAR).exists(),
        "nothing was written"
    );
}

// --- test 5: a resolved private address is refused before anything connects -

/// The policy judges the name; the downloader refuses what it resolves to:
/// a declared name resolving private is refused before any connection — the
/// listener is the witness that nothing was hit. The same shape with a
/// public resolution downloads.
#[tokio::test]
async fn a_resolved_private_address_is_refused_even_when_the_hostname_is_declared() {
    let content = b"public body".to_vec();
    let (_root, workspace) = workspace("private");
    let site = LocalSite::spawn(vec![("/data", route(content.as_slice(), 16))]).await;
    let net = transport(&site);

    let error = http_download(
        &policy(),
        &net,
        &workspace,
        "downloads/data.bin",
        "https://private-resolve.example.org/data",
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect_err("resolves private, so refused");
    assert!(
        matches!(
            error,
            DownloadError::Refused(FetchRefusal::DisallowedAddress { ref host })
                if host == "10.0.0.7"
        ),
        "the resolved address is named: {error:?}"
    );
    assert!(
        site.served().is_empty(),
        "refused before any connection: {:?}",
        site.served()
    );

    // The positive control: the same shape with a public resolution downloads.
    let outcome = http_download(
        &policy(),
        &net,
        &workspace,
        "downloads/data.bin",
        "https://public-resolve.example.org/data",
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect("public resolution downloads");
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(site.served(), vec!["/data".to_owned()]);
}
