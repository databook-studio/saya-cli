use super::ChatRequest;

/// A request carries no per-request output ceiling: the provider's configured
/// one — or the model's own default when none is configured — is the only
/// source, so the serialized projection never names one.
#[test]
fn a_request_carries_no_output_ceiling() {
    let json = serde_json::to_string(&ChatRequest::new("m", Vec::new())).expect("serializes");
    assert!(
        !json.contains("max_output_tokens"),
        "a request names no output ceiling: {json}"
    );
}
