struct TurnAndExtractionProvider {
    turn_step: Mutex<usize>,
    turn_steps: Vec<ChatResponse>,
    extraction_response: Result<ChatResponse, ProviderError>,
    extraction_calls: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for TurnAndExtractionProvider {
    fn name(&self) -> &str {
        "turn-and-extraction-provider"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        // The extraction request is the one whose system prompt identifies SAYA's
        // post-turn extractor (`build_extraction_prompt` opens with that line). A
        // turn request's system prompt is the connection context, which never
        // contains this phrase, so the two are distinguished by content — the one
        // stable marker the production prompt guarantees.
        let is_extraction = request
            .messages
            .first()
            .map(|m| m.content.contains("precision schema knowledge extractor"))
            .unwrap_or(false);
        if is_extraction {
            let mut calls = self.extraction_calls.lock().unwrap();
            *calls += 1;
            self.extraction_response.clone()
        } else {
            let mut step = self.turn_step.lock().unwrap();
            let idx = *step;
            *step += 1;
            if idx < self.turn_steps.len() {
                Ok(self.turn_steps[idx].clone())
            } else {
                Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
            }
        }
    }
}
