use crate::config::doctor_task::TaskCheck;
use saya_agent::{
    KnowledgeOutcome, LearningSkipReason, OverrideFindingDto, ProposedClaimDto,
    ProviderRecoveryPhase, ProviderRecoveryReason, SuppliedContractDto, TokenUsage, ToolEffect,
    UsageCall, read_only_permits,
};
use saya_config::OutputFormat;
use saya_types::{QueryResult, SchemaTree};
use serde::Serialize;
mod contract_view;
mod io_view;
mod render_clarification;
mod render_contract;
mod render_delta;
mod render_doctor;
mod render_io;
mod render_json;
// The scripted run's one terminal result: the text-mode warning shape and
// the json/ndjson `turn_result` rendering.
mod render_learned;
mod render_memory;
pub(crate) mod render_turn;
/// The shared tool-call grouper and shaper both adapters consume: the piped
/// text renderer buffers through it, the TUI transcript follows in slice 3.
pub(crate) mod tool_groups;
pub use contract_view::{
    ContractClaimView, ContractConflictView, ContractQueueItemView, ContractView,
};
pub use io_view::{ContractExportView, ContractImportClaimView, ContractImportView};
/// Re-exported for the TUI, which renders [`AgentEvent::ClarificationNeeded`]
/// as the transcript's Question block, and for the ask-path tests, which pin
/// the block text — one shaper, both surfaces (B3c).
pub(crate) use render_clarification::clarification_text;
/// Re-exported for the TUI, which renders [`AgentEvent::KnowledgeProposed`] in
/// `apply_event` and shares this shaper so the wording lives in one place.
pub(crate) use render_learned::knowledge_learned_text;
/// Re-exported for the TUI, which renders [`AgentEvent::KnowledgeOverridden`] in
/// `apply_event` and shares this shaper so the wording lives in one place (A1).
pub(crate) use render_memory::knowledge_overridden_text;
/// Re-exported for the TUI, which renders [`AgentEvent::KnowledgeSupplied`] in
/// `apply_event` and shares this shaper so the wording lives in one place.
pub(crate) use render_memory::knowledge_supplied_text;
/// Re-exported for the TUI, which renders [`AgentEvent::KnowledgeLearningDisabled`]
/// in `apply_event` and shares this shaper so the wording lives in one place.
pub(crate) use render_memory::learning_disabled_text;
/// Re-exported for the TUI, which renders [`AgentEvent::KnowledgeLearningSkipped`]
/// in `apply_event` and shares this shaper so the wording lives in one place
/// (packet-54).
pub(crate) use render_memory::learning_skipped_text;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFormat {
    Text,
    Json,
    Ndjson,
}

/// Stable machine label for one possible effect of an incomplete run step.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResumeEffectCode {
    WorkspaceWrite,
    Scratch,
    Runner,
    Interpreter,
    Fetch,
    ExternalSideEffect,
    LocalStateWrite,
    Unknown,
}
impl From<crate::cli::FormatArg> for RenderFormat {
    fn from(value: crate::cli::FormatArg) -> Self {
        match value {
            crate::cli::FormatArg::Text => Self::Text,
            crate::cli::FormatArg::Json => Self::Json,
            crate::cli::FormatArg::Ndjson => Self::Ndjson,
        }
    }
}
impl From<OutputFormat> for RenderFormat {
    fn from(value: OutputFormat) -> Self {
        match value {
            OutputFormat::Text => Self::Text,
            OutputFormat::Json => Self::Json,
            OutputFormat::Ndjson => Self::Ndjson,
        }
    }
}
/// How a scripted (`--turn-file`) turn ended, named in the one
/// `turn_result` terminal event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Completed,
    Failed,
    Cancelled,
    BudgetStopped,
}

include!("render_mod/terminal_event.rs");
include!("render_mod/render_entry.rs");
include!("render_mod/text_adapter.rs");
include!("render_mod/recovery_labels.rs");
#[cfg(test)]
#[path = "resume_event_tests.rs"]
mod resume_event_tests;

include!("render_mod/query_and_schema.rs");
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_terminal_strips_control_bytes_and_preserves_tabs_and_newlines() {
        let input = "hello\x1b[31mRED\x1b[0m\tworld\n\x1b]0;pwned\x07\r\x7f\u{0080}\u{009f}";
        let sanitized = sanitize_terminal(input);
        assert_eq!(sanitized, "hello[31mRED[0m\tworld\n]0;pwned");
        assert!(!sanitized.contains('\x1b'));
        assert!(!sanitized.contains('\x07'));
        assert!(!sanitized.contains('\r'));
        assert!(!sanitized.contains('\x7f'));
        assert!(!sanitized.contains('\u{0080}'));
        assert!(!sanitized.contains('\u{009f}'));
    }

    #[test]
    fn test_text_render_sanitizes_terminal_control_sequences() {
        let raw_text = "col1\x1b[31mRED\x1b[0m\tcol2\x1b]0;pwned\x07";
        let event = TerminalEvent::QueryResult {
            result: QueryResult {
                columns: vec!["col1".into(), "col2".into()],
                rows: vec![serde_json::json!([raw_text, "ok"])],
                row_count: 1,
                truncated: false,
                executed_sql: "SELECT 1".into(),
            },
        };

        let rendered_text = render_event(&event, RenderFormat::Text);
        assert!(!rendered_text.stdout.contains('\x1b'));
        assert!(!rendered_text.stdout.contains('\x07'));
        assert!(rendered_text.stdout.contains("col1[31mRED[0m"));
        assert!(rendered_text.stdout.contains("pwned"));
        assert!(rendered_text.stdout.contains('\t'));
        assert!(rendered_text.stdout.contains('\n'));

        let rendered_json = render_event(&event, RenderFormat::Json);
        assert!(rendered_json.stdout.contains("\\u001b[31mRED\\u001b[0m"));
        assert!(rendered_json.stdout.contains("\\u001b]0;pwned\\u0007"));
    }

    #[test]
    fn test_assistant_text_and_delta_sanitizes_control_sequences() {
        let raw = "\x1b[31mRED\x1b[0m\x1b]0;pwned\x07";
        let delta_rendered = render_delta::text(raw);
        assert!(!delta_rendered.stdout.contains('\x1b'));
        assert!(!delta_rendered.stdout.contains('\x07'));
        assert_eq!(delta_rendered.stdout, "[31mRED[0m]0;pwned");

        let event = TerminalEvent::AssistantText {
            text: raw.to_string(),
        };
        let text_rendered = render_event(&event, RenderFormat::Text);
        assert!(!text_rendered.stdout.contains('\x1b'));
        assert!(!text_rendered.stdout.contains('\x07'));

        let json_rendered = render_event(&event, RenderFormat::Json);
        assert!(json_rendered.stdout.contains("\\u001b"));
    }

    #[test]
    fn test_contract_remembered_replaced_render() {
        let event = TerminalEvent::ContractRemembered {
            claim_id: "ki-123".to_string(),
            object: "pagila.public.rental".to_string(),
            kind: "grain".to_string(),
            value: "one row per rental per day".to_string(),
            column: None,
            previous: Some("one row per rental".to_string()),
            action: "replaced".to_string(),
            status: "confirmed".to_string(),
        };
        let text_rendered = render_event(&event, RenderFormat::Text);
        assert_eq!(
            text_rendered.stdout,
            "replaced grain for pagila.public.rental: \"one row per rental\" -> \"one row per rental per day\"\n"
        );

        let event_col = TerminalEvent::ContractRemembered {
            claim_id: "ki-456".to_string(),
            object: "pagila.public.rental".to_string(),
            kind: "column-role".to_string(),
            value: "event_time".to_string(),
            column: Some("rental_date".to_string()),
            previous: Some("timestamp".to_string()),
            action: "replaced".to_string(),
            status: "confirmed".to_string(),
        };
        let text_rendered_col = render_event(&event_col, RenderFormat::Text);
        assert_eq!(
            text_rendered_col.stdout,
            "replaced column-role for pagila.public.rental (col: rental_date): \"timestamp\" -> \"event_time\"\n"
        );
    }

    #[test]
    fn provider_recovery_renders_the_fixed_tool_protocol_reason() {
        let rendered = render_event(
            &TerminalEvent::ProviderRecovery {
                phase: ProviderRecoveryPhase::NotRetried,
                reason: ProviderRecoveryReason::ToolCallProtocol,
                attempt: 1,
                limit: 4,
            },
            RenderFormat::Text,
        );

        assert_eq!(
            rendered.stdout,
            "provider recovery: not retried attempt 1 of 4 (invalid tool-call identifiers)\n"
        );
    }
}
