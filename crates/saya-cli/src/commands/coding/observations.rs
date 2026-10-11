use super::{CodingError, NativeCodingJob, context};
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMetric, AnsweringObservation, CancellationReason,
    ContextCompaction,
};
use saya_harness::coding::coordinator::{ChildEvent, ChildEventKind};
use saya_types::{AgentId, InvocationId};
use serde_json::json;
use std::sync::atomic::Ordering;
pub(super) struct NativeSink<'a> {
    pub(super) job: &'a NativeCodingJob,
    pub(super) downstream: &'a dyn AgentEventSink,
}
#[async_trait]
impl AgentEventSink for NativeSink<'_> {
    fn observe_compaction(&self, observation: ContextCompaction) {
        self.job.record_compaction(None, None, observation);
        self.downstream.observe_compaction(observation);
    }
    async fn emit(&self, event: AgentEvent) {
        self.downstream.emit(event).await;
    }
    async fn observe_answering(&self, observation: AnsweringObservation) {
        self.downstream.observe_answering(observation).await;
    }
    async fn metric(&self, metric: AgentMetric) {
        self.downstream.metric(metric).await;
    }
}
impl NativeCodingJob {
    fn record_compaction(
        &self,
        agent: Option<&AgentId>,
        invocation: Option<&InvocationId>,
        observation: ContextCompaction,
    ) {
        let record = self.observation_sequence.fetch_add(1, Ordering::AcqRel);
        let value = json!({"job_id":self.job_id(),"agent_id":agent,"invocation_id":invocation,"epoch":self.epoch,"record_id":record,"authority_policy_sha256":self.policy_digest,"observation":observation,"source":"actual_agent_context_mutation","byte_measure":"context_estimator_not_wire_or_tokens"});
        let result = serde_json::to_vec(&value)
            .map_err(CodingError::owner)
            .and_then(|bytes| {
                context::append_line(&self.records.join("compactions.jsonl"), &bytes, 1024 * 1024)
            });
        if result.is_err() {
            self.observation_failed.store(true, Ordering::Release);
            self.cancellation
                .cancel_with_reason(CancellationReason::OwnerFailure);
        }
    }
    pub(super) fn forward_progress(&self, event: ChildEvent) {
        if let ChildEventKind::Compaction { observation } = event.kind() {
            self.record_compaction(
                Some(event.agent_id()),
                Some(event.invocation_id()),
                *observation,
            );
        }
        let _ = self.progress.send(event);
    }
    pub(super) async fn record_observation_boundary(
        &self,
        phase: &str,
        complete: bool,
    ) -> Result<(), CodingError> {
        let budget = self.ledger.snapshot().await;
        let value = json!({"job_id":self.job_id(),"epoch":self.epoch,"phase":phase,"complete":complete&&!self.observation_failed.load(Ordering::Acquire),"compaction_records":self.observation_sequence.load(Ordering::Acquire).saturating_sub(1),"logical_requests":budget.logical_requests,"transport_attempts":budget.transport_attempts,"tool_calls":budget.tool_calls,"unsettled_reservations":budget.unsettled_reservations});
        context::append_line(
            &self.records.join("observations.jsonl"),
            &serde_json::to_vec(&value).map_err(CodingError::owner)?,
            1024 * 1024,
        )
    }
}
