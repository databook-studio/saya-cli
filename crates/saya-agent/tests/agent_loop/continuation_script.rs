fn continuation_script(
    requests: &Arc<Mutex<Vec<ChatRequest>>>,
    always_truncate: bool,
    partial_text: &str,
    partial_tool_json: Vec<String>,
) -> ContinuationScriptProvider {
    ContinuationScriptProvider {
        requests: requests.clone(),
        calls: Mutex::new(0),
        always_truncate,
        partial_text: partial_text.into(),
        partial_tool_json,
    }
}
