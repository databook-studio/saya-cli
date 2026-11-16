/// Content-free fact from an actual context compaction. Bytes use the loop's
/// context-size estimator, not exact wire bytes or provider token counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ContextCompaction {
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub evicted_tool_groups: u64,
    pub truncated_tool_messages: u64,
    pub protected_prefix_messages: u64,
    pub protected_prefix_unchanged: bool,
}

use crate::{AgentEvent, AgentMetric, AnsweringObservation};
use async_trait::async_trait;
#[async_trait]
pub trait AgentEventSink: Send + Sync {
    /// Called synchronously after mutation, never for a dry-run or a no-op.
    /// Observers own bounded persistence/delivery and must surface missing facts.
    fn observe_compaction(&self, _observation: ContextCompaction) {}

    /// Content-free metadata from the actual tool result owner.
    async fn observe_tool(&self, _diagnostic: ToolDiagnostic) {}

    async fn emit(&self, event: AgentEvent);

    /// A background shell run ended and the loop told the model so. `text` is
    /// the notice the next request carries. A display surface shows it as an
    /// activity line; the default drops it, because the notice is already in
    /// the model's conversation.
    async fn background_exit(&self, _text: &str) {}

    /// Receives source-owned admission counts, separate from display events.
    /// A sink that cannot persist an observation should cancel its shared token
    /// with `CancellationReason::OwnerFailure` before returning.
    async fn observe_answering(&self, _observation: AnsweringObservation) {}

    /// Receives fixed source-owned metrics, separate from display events.
    async fn metric(&self, _metric: AgentMetric) {}
}
pub struct NoopEventSink;
#[async_trait]
impl AgentEventSink for NoopEventSink {
    async fn emit(&self, _: AgentEvent) {}
}

/// Fixed outcome categories; no model or executor error text is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDiagnosticStatus {
    Requested,
    Completed,
    Denied,
    Failed,
    Refused,
}

/// The longest reason text a diagnostic carries: the tool-result error text the
/// model received, cut on a UTF-8 boundary with a trailing marker.
pub const MAX_TOOL_REASON_BYTES: usize = 1024;

/// The longest one call's whole payload may be: the projected diagnostic and
/// the transcript that rides beside it. A child's diagnostic travels as one
/// child event, whose own bound is 8 KiB (`MAX_CHILD_EVENT_BYTES`), and the
/// identity envelope around it is a few hundred bytes; the transcript rides in
/// the same in-process message, so a call inside this bound always fits — a
/// transcript can never be the reason a child's event is rejected. The
/// transcript is dropped instead, and the call's metadata and reason text
/// survive.
pub const MAX_TOOL_DIAGNOSTIC_BYTES: usize = 7 * 1024;

/// Appended when a reason text is cut to [`MAX_TOOL_REASON_BYTES`].
pub const TOOL_REASON_TRUNCATION_MARKER: &str = "...[truncated]";

/// Bounds the tool-result error text one diagnostic records. Unlike the closed
/// failure class, this text is the executor's or the policy's own wording, so
/// it is cut to [`MAX_TOOL_REASON_BYTES`] on a UTF-8 boundary, with a trailing
/// marker when the cut happened. Redaction is the durable writer's job.
pub fn bound_tool_reason(text: &str) -> String {
    if text.len() <= MAX_TOOL_REASON_BYTES {
        return text.to_owned();
    }
    let mut end = MAX_TOOL_REASON_BYTES - TOOL_REASON_TRUNCATION_MARKER.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TOOL_REASON_TRUNCATION_MARKER}", &text[..end])
}

/// Detail for a failed, denied or refused call: a digest of the arguments the
/// model sent, the closed failure class, that class's closed rendering, and the
/// bounded tool-result error text the model received. The digest is the only
/// form of the arguments retained; the class names are a fixed vocabulary; the
/// reason text is the one field that carries the executor's or the policy's own
/// wording, and the durable writer redacts and re-bounds it before it is stored.
/// It is named `reason_text`, never `reason`: `reason` stays the closed outcome
/// word every row carries.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ToolDiagnosticDetail {
    arguments_sha256: String,
    failure_class: &'static str,
    error_text: &'static str,
    reason_text: String,
}

impl ToolDiagnosticDetail {
    /// The lowercase-hex SHA-256 of the serialized call arguments.
    pub fn arguments_sha256(&self) -> &str {
        &self.arguments_sha256
    }
    /// The closed failure class, from a fixed vocabulary.
    pub fn failure_class(&self) -> &'static str {
        self.failure_class
    }
    /// The closed rendering of [`Self::failure_class`]; never provider text.
    pub fn error_text(&self) -> &'static str {
        self.error_text
    }
    /// The tool-result error text the model received for this call, bounded to
    /// [`MAX_TOOL_REASON_BYTES`]. This is the rule's own wording — which policy
    /// or executor refused the call — not a closed rendering of the class.
    pub fn reason_text(&self) -> &str {
        &self.reason_text
    }
}

/// The closed failure class and its closed rendering for one outcome status.
/// `None` for a call that succeeded or was only requested.
fn failure_wording(status: ToolDiagnosticStatus) -> Option<(&'static str, &'static str)> {
    match status {
        ToolDiagnosticStatus::Requested | ToolDiagnosticStatus::Completed => None,
        ToolDiagnosticStatus::Denied => {
            Some(("approval_denied", "the call was denied by approval policy"))
        }
        ToolDiagnosticStatus::Refused => Some((
            "admission_refused",
            "the effect admission refused the call before execution",
        )),
        ToolDiagnosticStatus::Failed => {
            Some(("executor_error", "the tool executor returned an error"))
        }
    }
}

/// A bounded source-owned tool name and result category, plus the bounded
/// transcript of the call itself. Fields are private and construction is
/// restricted to the agent loop; no deserializer admits content.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ToolDiagnostic {
    tool: String,
    status: ToolDiagnosticStatus,
    reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<ToolDiagnosticDetail>,
    /// The bounded transcript of the call, when the loop recorded one. It is
    /// evidence, not a projection, and it never serializes: a child's projected
    /// event is the serialized diagnostic, and that projection carries progress
    /// and bounded outcome words only — never the arguments a child sent or the
    /// result it received. The job's evidence writer reads it off the value with
    /// [`Self::transcript`] and redacts and re-bounds every field itself.
    #[serde(skip)]
    transcript: Option<crate::ToolCallTranscript>,
}
impl ToolDiagnostic {
    pub(crate) fn new(
        definition: Option<&crate::ToolDefinition>,
        status: ToolDiagnosticStatus,
    ) -> Self {
        let tool = definition
            .map(|definition| definition.name.as_str())
            .filter(|name| {
                !name.is_empty()
                    && name.len() <= 64
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            })
            .unwrap_or("unknown_tool")
            .to_owned();
        let reason = match status {
            ToolDiagnosticStatus::Requested => "dispatch_requested",
            ToolDiagnosticStatus::Completed => "executor_completed",
            ToolDiagnosticStatus::Denied => "policy_denied",
            ToolDiagnosticStatus::Failed => "tool_failed",
            ToolDiagnosticStatus::Refused => "execution_refused",
        };
        Self {
            tool,
            status,
            reason,
            detail: None,
            transcript: None,
        }
    }

    /// Attaches the bounded transcript of this call: the id the provider gave
    /// it, the arguments the model sent, the result the tool returned and how
    /// long the call took. The durable writer redacts and re-bounds every field
    /// before storing it. The transcript never serializes, so this bounds what
    /// one call's payload is — the projected diagnostic plus the transcript
    /// that rides beside it in the same child message. A payload past
    /// [`MAX_TOOL_DIAGNOSTIC_BYTES`] drops the transcript whole, so the call
    /// keeps its metadata and its reason text whatever the content's size.
    /// Attach the detail first: this reads the diagnostic as it stands.
    pub(crate) fn with_transcript(mut self, transcript: crate::ToolCallTranscript) -> Self {
        self.transcript = Some(transcript);
        if self.payload_bytes() > MAX_TOOL_DIAGNOSTIC_BYTES {
            self.transcript = None;
        }
        self
    }

    /// The serialized size of one call's whole payload: the projected
    /// diagnostic, which never carries the transcript, and the transcript that
    /// rides beside it. The two are measured apart because the projection
    /// excludes the transcript by construction.
    fn payload_bytes(&self) -> usize {
        let projected = serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len());
        let transcript = self.transcript.as_ref().map_or(0, |transcript| {
            serde_json::to_vec(transcript).map_or(usize::MAX, |bytes| bytes.len())
        });
        projected.saturating_add(transcript)
    }

    /// The bounded transcript of this call, when the loop recorded one.
    pub fn transcript(&self) -> Option<&crate::ToolCallTranscript> {
        self.transcript.as_ref()
    }

    /// Attaches the failure detail for a call that did not succeed: the digest
    /// of the serialized arguments the model sent, the closed failure class,
    /// its closed rendering, and the bounded tool-result error text the model
    /// received. A succeeding or merely requested call keeps `None`, so the
    /// durable record grows only where a postmortem needs to explain an
    /// outcome — and a succeeding result is never serialized to be discarded.
    pub(crate) fn with_failure_detail(
        mut self,
        arguments_json: &str,
        reason_text: impl FnOnce() -> String,
    ) -> Self {
        if let Some((failure_class, error_text)) = failure_wording(self.status) {
            self.detail = Some(ToolDiagnosticDetail {
                arguments_sha256: saya_types::sha256_hex(arguments_json.as_bytes()),
                failure_class,
                error_text,
                reason_text: bound_tool_reason(&reason_text()),
            });
        }
        self
    }

    /// The text of a non-succeeding tool result as the model received it: the
    /// result's own `error` string when it carries one, else the whole result.
    pub(crate) fn result_error_text(result: &serde_json::Value) -> String {
        result
            .get("error")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| result.to_string())
    }

    /// The bounded failure detail, when this diagnostic records one.
    pub fn detail(&self) -> Option<&ToolDiagnosticDetail> {
        self.detail.as_ref()
    }
    /// These interceptors remain reachable even when omitted from definitions.
    /// Only fixed protocol names bypass definition-name validation.
    pub(crate) fn special(
        name: &str,
        definitions: &[crate::ToolDefinition],
        status: ToolDiagnosticStatus,
    ) -> Self {
        let mut diagnostic = Self::new(
            definitions
                .iter()
                .find(|definition| definition.name == name),
            status,
        );
        if matches!(name, "request_clarification" | "designate_answer") {
            diagnostic.tool = name.to_owned();
        }
        diagnostic
    }

    pub fn tool(&self) -> &str {
        &self.tool
    }
    pub fn status(&self) -> ToolDiagnosticStatus {
        self.status
    }
    pub fn reason(&self) -> &'static str {
        self.reason
    }

    pub(crate) fn result_status(
        result: &serde_json::Value,
        succeeded: bool,
    ) -> ToolDiagnosticStatus {
        if !succeeded {
            ToolDiagnosticStatus::Failed
        } else if result.get("outcome").and_then(serde_json::Value::as_str) == Some("refused") {
            ToolDiagnosticStatus::Refused
        } else {
            ToolDiagnosticStatus::Completed
        }
    }
}

#[cfg(test)]
#[path = "event_sink_tests.rs"]
mod event_sink_tests;
