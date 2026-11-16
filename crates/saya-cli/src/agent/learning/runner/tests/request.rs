use super::*;

/// The extraction request carries both JSON intent (`ResponseFormat::JsonObject`)
/// and the honest effort lever (`ReasoningEffort::Minimal`) on the
/// `ChatRequest` — the provider-neutral ask. This asserts the *request*, not
/// the wire: the OpenAI-family provider **drops** `Minimal` at the wire (see
/// `providers/openai.rs`), because the `"minimal"` spelling is not
/// universally honoured; Ollama/Anthropic/Gemini translate it. So this test
/// pins that the extraction call *asks* for minimal effort and JSON mode,
/// while the wire spelling each provider emits is pinned in `openai.rs`.
/// This is the one call in saya that sets them — the main loop's request
/// does not (see `receive.rs`).
#[tokio::test]
async fn run_extraction_sets_json_mode_and_minimal_effort_on_the_provider_request() {
    let identity = test_identity("analytics");
    let registry = test_registry("analytics", &identity);
    let root = temp_root("json_mode");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let receipt = RecallReceipt::ran_empty(false);

    let mut object_table = TurnObjectTable::new();
    object_table.register("analytics", "raw.orders", &["status".into()]);

    let record = TurnRecord {
        prompt: "what is orders status".into(),
        assistant_answer: "status is pending".into(),
        object_table,
        user_corrections: Vec::new(),
        override_findings: Vec::new(),
        supplied_claims: Vec::new(),
        omitted: 0,
    };

    let provider = RecordingProvider {
        response_text: r#"{"proposals": []}"#.into(),
        captured: Mutex::new(None),
    };

    let outcome = run_extraction(&provider, "glm-5.2", &record, &registry, &store, &receipt).await;
    let res = outcome.dtos.expect("extraction succeeds");

    assert!(res.is_empty(), "empty-proposal response stores nothing");
    let sent = provider
        .captured
        .lock()
        .unwrap()
        .take()
        .expect("a request was sent to the provider");
    assert_eq!(
        sent.response_format,
        ResponseFormat::JsonObject,
        "the extraction call must request JSON mode"
    );
    assert_eq!(
        sent.reasoning_effort,
        ReasoningEffort::Minimal,
        "the extraction call must request minimal effort"
    );
    let _ = fs::remove_dir_all(root);
}

/// A provider that returns a response carrying a configurable usage, so the
/// accounting can assert what the extraction call reported.
pub(super) struct UsageExtractionProvider {
    pub(super) response_text: String,
    pub(super) usage: TokenUsage,
}

#[async_trait]
impl ChatProvider for UsageExtractionProvider {
    fn name(&self) -> &str {
        "usage-provider"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut response = ChatResponse::new(ChatMessage::text("assistant", &self.response_text));
        response.usage = Some(self.usage);
        Ok(response)
    }
}
