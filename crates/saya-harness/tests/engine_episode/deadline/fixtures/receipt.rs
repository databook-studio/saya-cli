struct ReceiptThenTool {
    journal_path: PathBuf,
    kept_journal: PathBuf,
    calls: AtomicUsize,
    cancel_first: Option<CancellationToken>,
}

#[async_trait]
impl ChatProvider for ReceiptThenTool {
    fn name(&self) -> &str {
        "receipt-then-tool"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        if first {
            if let Some(cancellation) = &self.cancel_first {
                cancellation.cancel_with_reason(CancellationReason::User);
            }
            fs::copy(&self.journal_path, &self.kept_journal).unwrap();
            fs::remove_file(&self.journal_path).unwrap();
            fs::create_dir(&self.journal_path).unwrap();
        } else {
            return Ok(ChatResponse::new(ChatMessage::text("assistant", "done")));
        }
        let mut response = ChatResponse::new(ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![call("probe")],
            tool_call_id: None,
        });
        response.usage = Some(TokenUsage::new(1, 0));
        Ok(response)
    }
}
