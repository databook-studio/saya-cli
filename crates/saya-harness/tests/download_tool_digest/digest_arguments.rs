use super::*;

/// A no-digest call over an existing destination is create-only: it refuses
/// before the transport is ever asked, and the file is untouched.
#[tokio::test]
async fn a_call_without_a_digest_over_an_existing_destination_refuses_before_the_transport() {
    let on_wire = b"the body the call wanted".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("create-only");
    seed_destination(&workspace, &existing);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget.clone());
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({"url": url("/doc"), "destination": DEST}),
        )
        .await
        .expect_err("create-only refuses an existing destination");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("already exists"),
        "the create-only refusal names the collision: {detail}"
    );
    assert_eq!(on_disk(&workspace), existing, "the file is untouched");
    assert!(
        calls.lock().expect("calls").is_empty(),
        "the refusal happens before any transport call"
    );
    assert_eq!(budget.consumed(), 0, "no budget was spent");
}

/// A malformed digest — not exactly 64 lowercase hex characters — refuses
/// at the argument boundary, before any transport call or budget use, with
/// an error that names the shape but never reveals or computes a digest.
#[tokio::test]
async fn a_malformed_digest_refuses_before_any_transport_call_or_budget_use() {
    let on_wire = b"never fetched".to_vec();
    let (_root, workspace) = workspace("malformed");
    for bad in ["ZZ", &"a".repeat(63), &"A".repeat(64), &"g".repeat(64)] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let budget = DownloadBudget::default();
        let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
        let tools = tools(net, workspace.clone(), budget.clone());
        let error = tools
            .execute(
                "http_download",
                serde_json::json!({
                    "url": url("/doc"),
                    "destination": DEST,
                    "expected_digest": bad,
                }),
            )
            .await
            .expect_err("a malformed digest is refused");
        let ToolError::Fetch(detail) = error else {
            panic!("the lane's typed error, got: {error:?}")
        };
        assert!(
            detail.contains("expected_digest"),
            "the refusal names the argument: {detail}"
        );
        assert!(
            !detail.contains(&digest(&on_wire)),
            "the refusal never computes a digest: {detail}"
        );
        assert!(
            calls.lock().expect("calls").is_empty(),
            "the malformed digest never reaches the transport"
        );
        assert_eq!(budget.consumed(), 0, "no budget was spent");
    }
    assert!(
        !workspace.root().join(DEST).exists(),
        "nothing landed anywhere"
    );
}

/// A non-string digest is its own typed argument error, like the guarded
/// file tools', and never reaches the transport.
#[tokio::test]
async fn a_non_string_digest_is_a_typed_argument_error() {
    let on_wire = b"never fetched".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("non-string");
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
                "expected_digest": 7,
            }),
        )
        .await
        .expect_err("a non-string digest is a typed argument error");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("expected_digest must be a string"),
        "the typed argument error: {detail}"
    );
    assert!(
        calls.lock().expect("calls").is_empty(),
        "the argument error never reaches the transport"
    );
    assert_eq!(on_disk(&workspace), existing, "the file is untouched");
}

/// An explicit null digest is absent — the same convention the guarded file
/// tools' argument parsers follow — so over an existing destination the
/// create-only refusal applies.
#[tokio::test]
async fn an_explicit_null_digest_is_the_create_form_over_an_existing_destination() {
    let on_wire = b"never fetched".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("null-digest");
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
                "expected_digest": serde_json::Value::Null,
            }),
        )
        .await
        .expect_err("null is absent, so create-only refuses the existing destination");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("already exists"),
        "the create-only refusal: {detail}"
    );
    assert!(
        calls.lock().expect("calls").is_empty(),
        "the create-only refusal happens before the transport"
    );
    assert_eq!(on_disk(&workspace), existing, "the file is untouched");
}

/// The run lane's definition advertises the optional argument with the same
/// name and shape the guarded file tools use, and the required list is
/// unchanged.
#[test]
fn the_run_lane_definition_advertises_the_optional_digest() {
    let definition = http_download_definition();
    let properties = definition.parameters["properties"]
        .as_object()
        .expect("the schema has properties");
    let declared = properties
        .get("expected_digest")
        .expect("expected_digest is declared");
    assert_eq!(declared["type"], "string", "the digest is a string");
    let description = declared["description"].as_str().expect("described");
    assert!(
        description.contains("never invent or guess"),
        "the description forbids invented digests: {description}"
    );
    assert!(
        description.contains("read it first"),
        "the description requires observing the digest from a read: {description}"
    );
    assert_eq!(
        definition.parameters["required"],
        serde_json::json!(["url", "destination"]),
        "the required list is unchanged: the digest stays optional"
    );
    assert_eq!(
        definition.parameters["additionalProperties"],
        serde_json::json!(false),
        "the schema stays closed"
    );
}
