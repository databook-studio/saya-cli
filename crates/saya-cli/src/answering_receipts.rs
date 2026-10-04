use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, AnsweringObservation, TokenUsage, UsageCall};
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct AnsweringReceipts(Mutex<Option<TokenUsage>>);

impl AnsweringReceipts {
    pub(crate) fn record(&self, event: &AgentEvent) {
        let usage = match event {
            AgentEvent::Usage {
                call: UsageCall::Answer,
                usage,
            }
            | AgentEvent::FailedAttemptUsage { usage } => usage,
            _ => return,
        };
        let mut total = self.0.lock().expect("answering receipts");
        add_usage(total.get_or_insert_default(), *usage);
    }

    pub(crate) fn take(&self) -> Option<TokenUsage> {
        self.0.lock().expect("answering receipts").take()
    }
}

fn add_usage(total: &mut TokenUsage, usage: TokenUsage) {
    total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
    sum_optional(&mut total.cached_input_tokens, usage.cached_input_tokens);
    sum_optional(
        &mut total.cache_creation_input_tokens,
        usage.cache_creation_input_tokens,
    );
    sum_optional(&mut total.reasoning_tokens, usage.reasoning_tokens);
}

fn sum_optional(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or(0).saturating_add(value));
    }
}

pub(crate) struct AnsweringReceiptSink<'a> {
    inner: &'a dyn AgentEventSink,
    receipts: AnsweringReceipts,
}

impl<'a> AnsweringReceiptSink<'a> {
    pub(crate) fn new(inner: &'a dyn AgentEventSink) -> Self {
        Self {
            inner,
            receipts: AnsweringReceipts(Mutex::new(None)),
        }
    }

    pub(crate) fn take(&self) -> Option<TokenUsage> {
        self.receipts.take()
    }
}

#[async_trait]
impl AgentEventSink for AnsweringReceiptSink<'_> {
    async fn emit(&self, event: AgentEvent) {
        self.receipts.record(&event);
        self.inner.emit(event).await;
    }

    async fn observe_answering(&self, observation: AnsweringObservation) {
        self.inner.observe_answering(observation).await;
    }
}
