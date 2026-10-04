use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMetric, AnsweringObservation, TokenUsage, UsageCall,
};
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

    async fn metric(&self, metric: AgentMetric) {
        self.inner.metric(metric).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use saya_agent::{
        AgentAttemptOutcome, AgentMetric, AgentMetricCount, AgentMetricOutcome, AgentMetricPhase,
        AgentMetricUseful,
    };

    #[derive(Debug, Clone, PartialEq)]
    enum SinkRecord {
        Event(AgentEvent),
        Metric(AgentMetric),
    }

    struct RecordingSink(Mutex<Vec<SinkRecord>>);

    impl RecordingSink {
        fn records(&self) -> Vec<SinkRecord> {
            self.0.lock().expect("recording sink lock").clone()
        }
    }

    #[async_trait]
    impl AgentEventSink for RecordingSink {
        async fn emit(&self, event: AgentEvent) {
            self.0
                .lock()
                .expect("recording sink lock")
                .push(SinkRecord::Event(event));
        }

        async fn metric(&self, metric: AgentMetric) {
            self.0
                .lock()
                .expect("recording sink lock")
                .push(SinkRecord::Metric(metric));
        }
    }

    #[tokio::test]
    async fn metric_reaches_inner_once_unchanged_and_in_order() {
        let sink = RecordingSink(Mutex::new(Vec::new()));
        let wrapper = AnsweringReceiptSink::new(&sink);
        wrapper.emit(AgentEvent::turn_started()).await;
        wrapper
            .metric(AgentMetric::Count(AgentMetricCount::RetryAttemptStarted))
            .await;
        wrapper.emit(AgentEvent::assistant_text("partial")).await;
        assert_eq!(
            sink.records(),
            vec![
                SinkRecord::Event(AgentEvent::TurnStarted),
                SinkRecord::Metric(AgentMetric::Count(AgentMetricCount::RetryAttemptStarted)),
                SinkRecord::Event(AgentEvent::AssistantText {
                    text: "partial".to_string(),
                }),
            ],
            "exactly one metric, unchanged, between the two emitted events"
        );
    }

    #[tokio::test]
    async fn metrics_leave_receipt_sums_unchanged() {
        let usage = || AgentEvent::usage(UsageCall::Answer, TokenUsage::new(2, 5));
        let plain_sink = RecordingSink(Mutex::new(Vec::new()));
        let plain = AnsweringReceiptSink::new(&plain_sink);
        plain.emit(usage()).await;
        plain.emit(usage()).await;

        let metrics_sink = RecordingSink(Mutex::new(Vec::new()));
        let with_metrics = AnsweringReceiptSink::new(&metrics_sink);
        with_metrics.emit(usage()).await;
        with_metrics
            .metric(AgentMetric::Phase {
                phase: AgentMetricPhase::ProviderEstablishment,
                elapsed_ns: 1,
                outcome: AgentMetricOutcome::Succeeded,
            })
            .await;
        with_metrics.emit(usage()).await;
        with_metrics
            .metric(AgentMetric::Useful(AgentMetricUseful::SuccessfulTool))
            .await;
        with_metrics
            .metric(AgentMetric::UsageCoverage {
                attempt_outcome: AgentAttemptOutcome::Succeeded,
                reported: true,
            })
            .await;

        let plain_total = plain.take();
        assert_eq!(plain_total, Some(TokenUsage::new(4, 10)), "usage sums");
        assert_eq!(
            with_metrics.take(),
            plain_total,
            "metrics must not enter receipts"
        );
    }
}
