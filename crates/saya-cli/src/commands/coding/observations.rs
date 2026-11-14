#[cfg(all(test, unix))]
#[path = "observations_tests.rs"]
mod observations_tests;

use super::{CodingError, NativeCodingJob, diagnostics};
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMetric, AgentMetricPhase, AnsweringObservation,
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
    async fn observe_tool(&self, diagnostic: saya_agent::ToolDiagnostic) {
        self.job.record_tool(None, None, &diagnostic);
        self.downstream.observe_tool(diagnostic).await;
    }

    fn observe_compaction(&self, observation: ContextCompaction) {
        self.job.record_compaction(None, None, observation);
        self.downstream.observe_compaction(observation);
    }
    async fn emit(&self, event: AgentEvent) {
        // The first text token of a turn may be the final response's, and the
        // turn's tool calls (if any) move the cache key anyway: start the
        // advisory readiness now so it finishes while the model streams.
        if let AgentEvent::AssistantText { .. } = &event {
            self.job.start_readiness_precompute();
        }
        self.downstream.emit(event).await;
    }
    async fn observe_answering(&self, observation: AnsweringObservation) {
        self.downstream.observe_answering(observation).await;
    }
    async fn metric(&self, metric: AgentMetric) {
        // The loop's own per-turn boundaries are timers, not authority: they
        // land in the bounded lifecycle summaries and change nothing else.
        if let AgentMetric::Phase {
            phase, elapsed_ns, ..
        } = &metric
        {
            let note = match phase {
                AgentMetricPhase::MessageAssembly => Some("message_assembly"),
                AgentMetricPhase::PromptProjection => Some("prompt_projection"),
                _ => None,
            };
            if let Some(note) = note {
                self.job
                    .lifecycle()
                    .note_elapsed(note, std::time::Duration::from_nanos(*elapsed_ns));
            }
        }
        self.downstream.metric(metric).await;
    }
}
/// Longest persisted error text: the closed rendering a failure class carries
/// is already far shorter, so this bound is a ceiling on the record shape, not
/// a truncation in practice.
const MAX_ERROR_TEXT_BYTES: usize = 200;

/// The bounded, redacted error text of one failure class. The text is closed
/// vocabulary and carries no paths, provider text or tool output; the store's
/// own redaction and bound run anyway, so a future wording cannot widen the
/// record by accident.
fn bounded_error_text(text: &str) -> String {
    let redacted = saya_types::redact(text);
    let mut end = redacted.len().min(MAX_ERROR_TEXT_BYTES);
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    redacted[..end].to_owned()
}

/// The redacted, bounded `reason_text` of one failed call: the tool-result error
/// text the model received. Unlike the closed `error_text`, this is the
/// executor's or the policy's own wording, so it is the store's redaction — not
/// a closed vocabulary — that keeps credentials out, and the same 1,024-byte
/// bound the diagnostic applied is re-applied after redaction, which can
/// lengthen the text.
fn bounded_reason_text(text: &str) -> String {
    saya_agent::bound_tool_reason(&saya_types::redact(text))
}

/// The unix-millisecond stamp of one durable record.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

impl NativeCodingJob {
    fn record_tool(
        &self,
        agent: Option<&AgentId>,
        invocation: Option<&InvocationId>,
        diagnostic: &saya_agent::ToolDiagnostic,
    ) {
        // One tool event's attach, append and marker check is one journal step:
        // holding the lock across them keeps the budget's call ordinals in the
        // order the records land, so no call can record a body after the marker
        // named the first dropped body.
        let _journal = self
            .transcript_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut value = json!({"job_id": self.job_id(), "agent_id": agent, "invocation_id": invocation,
            "epoch": self.epoch, "tool": diagnostic.tool(),
            "status": diagnostic.status(), "reason": diagnostic.reason(), "source": "actual_tool_loop"});
        // A failed, denied or refused call also records when it happened, the
        // digest of what the model sent, the closed failure class, and
        // `reason_text` — the redacted, bounded tool-result error text the model
        // received, so a postmortem can name the rule that refused the call.
        //
        // `reason` keeps its historical meaning on every row: the closed outcome
        // word. The free wording lives under its own key so a reader that only
        // knows the word is not handed the executor's text.
        if let Some(detail) = diagnostic.detail() {
            value["at_unix_millis"] = json!(now_ms());
            value["arguments_sha256"] = json!(detail.arguments_sha256());
            value["failure_class"] = json!(detail.failure_class());
            value["error_text"] = json!(bounded_error_text(detail.error_text()));
            value["reason_text"] = json!(bounded_reason_text(detail.reason_text()));
        }
        // The call's own transcript — the id the provider gave it, the arguments
        // the model sent, the result the tool returned and how long it took —
        // redacted and re-bounded by the writer. Every call keeps its compact
        // facts; only the result body is dropped once the job's result-body
        // budget is spent. The row's metadata is written either way.
        if let Some(transcript) = diagnostic.transcript() {
            self.transcripts.attach(&mut value, transcript);
        }
        let result = serde_json::to_vec(&value)
            .map_err(|_| diagnostics::Fault::InvalidRecord)
            .and_then(|bytes| {
                diagnostics::append(&self.records, diagnostics::Stream::ToolEvents, &bytes)
            });
        if let Err(fault) = result {
            self.latch_diagnostic_fault(fault);
            return;
        }
        // Once a result body is dropped, exactly one row names the first call
        // that lost its body and how much body was recorded; every call before
        // and after it keeps its compact facts.
        if let Some(marker) = self
            .transcripts
            .take_marker(self.job_id().as_str(), self.epoch)
        {
            let result = serde_json::to_vec(&marker)
                .map_err(|_| diagnostics::Fault::InvalidRecord)
                .and_then(|bytes| {
                    diagnostics::append(&self.records, diagnostics::Stream::ToolEvents, &bytes)
                });
            if let Err(fault) = result {
                self.latch_diagnostic_fault(fault);
            }
        }
    }

    fn record_compaction(
        &self,
        agent: Option<&AgentId>,
        invocation: Option<&InvocationId>,
        observation: ContextCompaction,
    ) {
        let record = self.observation_sequence.fetch_add(1, Ordering::AcqRel);
        let value = json!({"job_id":self.job_id(),"agent_id":agent,"invocation_id":invocation,"epoch":self.epoch,"record_id":record,"authority_policy_sha256":self.policy_digest,"observation":observation,"source":"actual_agent_context_mutation","byte_measure":"context_estimator_not_wire_or_tokens"});
        let result = serde_json::to_vec(&value)
            .map_err(|_| diagnostics::Fault::InvalidRecord)
            .and_then(|bytes| {
                diagnostics::append(&self.records, diagnostics::Stream::Compactions, &bytes)
            });
        if let Err(fault) = result {
            self.latch_diagnostic_fault(fault);
        }
    }
    pub(super) fn forward_progress(&self, event: ChildEvent) {
        // A completion needs no steering note: the `helpers` call that launched
        // the child awaits that child's own completion and returns its durable
        // summary as the tool result, so the root already holds the result
        // before any later turn could read a note about it.
        if let ChildEventKind::ToolDiagnostic { diagnostic } = event.kind() {
            self.record_tool(
                Some(event.agent_id()),
                Some(event.invocation_id()),
                diagnostic,
            );
        }

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
        let result = serde_json::to_vec(&value)
            .map_err(|_| diagnostics::Fault::InvalidRecord)
            .and_then(|bytes| {
                diagnostics::append(&self.records, diagnostics::Stream::Observations, &bytes)
            });
        if let Err(fault) = result {
            self.latch_diagnostic_fault(fault);
        }
        result.map_err(CodingError::Diagnostic)
    }
}
