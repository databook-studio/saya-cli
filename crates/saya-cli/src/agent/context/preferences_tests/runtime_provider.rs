#[derive(Default)]
struct Captured {
    first_request: Mutex<Option<ChatRequest>>,
    request_count: Mutex<usize>,
}

impl Captured {
    fn first_messages(&self) -> Option<Vec<ChatMessage>> {
        self.first_request
            .lock()
            .unwrap()
            .clone()
            .map(|r| r.messages)
    }

    fn request_count(&self) -> usize {
        *self.request_count.lock().unwrap()
    }
}

struct RequestRecorder {
    captured: Arc<Captured>,
}

#[async_trait]
impl ChatProvider for RequestRecorder {
    fn name(&self) -> &str {
        "preference-context-recorder"
    }
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        *self.captured.request_count.lock().unwrap() += 1;
        let mut first = self.captured.first_request.lock().unwrap();
        if first.is_none() {
            *first = Some(request);
        }
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}
