use super::*;

// --- test 3: a refused redirect hop, with the body never read ---------------

/// A redirect to a refused destination is refused at that hop: the first hop's
/// 302 is served, the target is judged and refused, and the 302's body is
/// never read (zero pulls) nor is the refused target ever connected to.
#[tokio::test]
async fn a_redirect_to_a_refused_destination_is_refused_at_that_hop() {
    let (site, tool, pulls) = fixture(
        vec![(
            "/start",
            redirect("https://169.254.169.254/latest/meta-data/"),
        )],
        FetchLimits::default(),
    )
    .await;
    let error = tool
        .fetch("https://corpus.example.org/start")
        .await
        .expect_err("the redirect target is refused");
    assert!(
        matches!(
            error,
            FetchToolError::Refused(FetchRefusal::DisallowedAddress { ref host }) if host == "169.254.169.254"
        ),
        "typed refusal at the refused hop: {error:?}"
    );
    assert_eq!(
        site.served(),
        vec!["/start".to_owned()],
        "only the first hop was ever connected"
    );
    assert_eq!(
        pulls.load(Ordering::SeqCst),
        0,
        "the redirect hop's body is never read"
    );
}

// --- test 4: a resolved private address is refused on an allowlisted name ---

/// The policy's documented residual — it does no DNS — is closed at the tool:
/// a declared name resolving private is refused on every returned address
/// before anything connects, and the same name resolving public is fetched.
/// A name that must not resolve (`*.invalid`) fails as a typed error.
#[tokio::test]
async fn a_resolved_private_address_is_refused_even_when_the_hostname_is_allowlisted() {
    let (site, tool, _pulls) = fixture(
        vec![("/data", ok(vec![b"public body"]))],
        FetchLimits::default(),
    )
    .await;

    let error = tool
        .fetch("https://private-resolve.example.org/data")
        .await
        .expect_err("resolves private, so refused");
    assert!(
        matches!(
            error,
            FetchToolError::Refused(FetchRefusal::DisallowedAddress { ref host }) if host == "10.0.0.7"
        ),
        "the resolved address is named: {error:?}"
    );
    assert!(
        site.served().is_empty(),
        "refused before any connection: {:?}",
        site.served()
    );

    // The positive control: the same shape with a public resolution fetches.
    let outcome = tool
        .fetch("https://public-resolve.example.org/data")
        .await
        .expect("public resolution fetches");
    assert_eq!(outcome.block.body, "public body");
    assert_eq!(site.served(), vec!["/data".to_owned()]);

    // A name that must not resolve fails typed, never guessed at.
    let error = tool
        .fetch("https://unresolvable.invalid/data")
        .await
        .expect_err("no DNS entry");
    assert!(matches!(error, FetchToolError::Transport(_)), "{error:?}");
}
