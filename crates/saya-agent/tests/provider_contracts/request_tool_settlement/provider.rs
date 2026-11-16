use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use saya_agent::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderEvent,
    ProviderStream, TokenUsage,
};

use super::owner::Trace;

pub struct ScriptedProvider {
    scripts: std::sync::Mutex<VecDeque<Vec<Result<ProviderEvent, ProviderError>>>>,
    pub calls: AtomicUsize,
    pub trace: Trace,
    pub cancel_on_stream: Option<CancellationToken>,
}

impl ScriptedProvider {
    pub fn new(scripts: Vec<Vec<ProviderEvent>>, trace: Trace) -> Self {
        Self::new_results(
            scripts
                .into_iter()
                .map(|events| events.into_iter().map(Ok).collect())
                .collect(),
            trace,
        )
    }

    pub fn new_results(
        scripts: Vec<Vec<Result<ProviderEvent, ProviderError>>>,
        trace: Trace,
    ) -> Self {
        Self {
            scripts: std::sync::Mutex::new(scripts.into()),
            calls: AtomicUsize::new(0),
            trace,
            cancel_on_stream: None,
        }
    }

    pub fn cancel_on_stream(mut self, token: CancellationToken) -> Self {
        self.cancel_on_stream = Some(token);
        self
    }
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "n01-scripted-provider"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the loop uses the streaming provider path")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.trace.lock().unwrap().push("provider".to_owned());
        if let Some(token) = &self.cancel_on_stream {
            token.cancel();
        }
        let script = self.scripts.lock().unwrap().pop_front().unwrap_or_else(|| {
            vec![
                Ok(ProviderEvent::TextDelta("fallback answer".to_owned())),
                Ok(ProviderEvent::Done),
            ]
        });
        Ok(Box::pin(futures_util::stream::iter(script)))
    }
}

pub fn answer(text: &str) -> Vec<ProviderEvent> {
    vec![
        ProviderEvent::TextDelta(text.to_owned()),
        ProviderEvent::Done,
    ]
}

pub fn tool_calls(calls: Vec<saya_agent::ToolCall>) -> Vec<ProviderEvent> {
    vec![ProviderEvent::ToolCalls(calls), ProviderEvent::Done]
}

pub fn reported_answer(text: &str, input_tokens: u64, output_tokens: u64) -> Vec<ProviderEvent> {
    let mut usage = TokenUsage::default();
    usage.input_tokens = input_tokens;
    usage.output_tokens = output_tokens;
    vec![
        ProviderEvent::Usage(usage),
        ProviderEvent::TextDelta(text.to_owned()),
        ProviderEvent::Done,
    ]
}
