use super::*;

/// A matching digest replaces the destination with the streamed body, and
/// the result metadata keeps its shape: destination, bytes, sha256.
#[tokio::test]
async fn a_matching_digest_replaces_the_destination_and_the_result_shape_is_unchanged() {
    let on_wire = b"the replacement body".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("matching");
    seed_destination(&workspace, &existing);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget);
    let value = tools
        .execute(
            "http_download",
            serde_json::json!({
                "url": url("/doc"),
                "destination": DEST,
                "expected_digest": digest(&existing),
            }),
        )
        .await
        .expect("the observed digest authorizes the replacement");
    assert_eq!(
        value["destination"].as_str(),
        Some(DEST),
        "the result names the destination"
    );
    assert_eq!(
        value["bytes"].as_u64(),
        Some(on_wire.len() as u64),
        "the result counts the streamed bytes"
    );
    assert_eq!(
        value["sha256"].as_str(),
        Some(digest(&on_wire).as_str()),
        "the result carries the new digest"
    );
    assert_eq!(on_disk(&workspace), on_wire, "the body replaced the file");
    assert_eq!(
        calls.lock().expect("calls").clone(),
        vec![url("/doc")],
        "exactly the one request was served"
    );
}

/// A stale digest — not the destination's current content — refuses
/// `ContentChanged` at the guarded rename and leaves the file byte-for-byte
/// unchanged; the precondition is judged against the destination, so the
/// transport was asked and refused to publish.
#[tokio::test]
async fn a_stale_digest_refuses_and_leaves_the_destination_unchanged() {
    let on_wire = b"the body the call wanted".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("stale");
    seed_destination(&workspace, &existing);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget);
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({
                "url": url("/doc"),
                "destination": DEST,
                "expected_digest": digest(&on_wire),
            }),
        )
        .await
        .expect_err("the digest names other content than the destination holds");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("does not match the expected_digest"),
        "the guarded stale-digest refusal: {detail}"
    );
    assert!(
        !detail.contains(&digest(&on_wire)),
        "the refusal never echoes the supplied digest: {detail}"
    );
    assert!(
        !detail.contains(&digest(&existing)),
        "the refusal never leaks the destination's real digest: {detail}"
    );
    assert_eq!(on_disk(&workspace), existing, "the file is untouched");
    assert_eq!(
        calls.lock().expect("calls").clone(),
        vec![url("/doc")],
        "the precondition is judged at the rename, after the one request"
    );
}

/// An invented digest — 64 zeros, observed from no read of anything — is
/// refused at the guarded rename, and the refusal carries no 64-hex
/// substring at all: nothing a model can mine to learn the destination's
/// real digest and replace a file it never read.
#[tokio::test]
async fn an_invented_digest_cannot_learn_the_real_one() {
    let on_wire = b"the body the call wanted".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("invented");
    seed_destination(&workspace, &existing);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget);
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({
                "url": url("/doc"),
                "destination": DEST,
                "expected_digest": "0".repeat(64),
            }),
        )
        .await
        .expect_err("an invented digest is refused");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        !contains_64_hex(&detail),
        "the refusal never carries a digest-shaped substring: {detail}"
    );
    assert_eq!(on_disk(&workspace), existing, "the file is untouched");
}

/// A digest over a missing destination is a refusal, never creation: the
/// precondition can only be judged against bytes that exist.
#[tokio::test]
async fn a_digest_over_a_missing_destination_refuses_and_creates_nothing() {
    let on_wire = b"the body the call wanted".to_vec();
    let (_root, workspace) = workspace("missing");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget);
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({
                "url": url("/doc"),
                "destination": DEST,
                "expected_digest": digest(&on_wire),
            }),
        )
        .await
        .expect_err("a digest never authorizes creation");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("does not exist"),
        "the missing-destination refusal: {detail}"
    );
    assert!(!workspace.root().join(DEST).exists(), "nothing was created");
}
