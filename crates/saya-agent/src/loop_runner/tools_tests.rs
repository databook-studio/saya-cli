//! The side-effect guard and the tool-message cap: the S2 loop seam's pins.
//!
//! Phase 7 packet 2: a failed tool's human-facing summary names *why* it
//! failed, so a safety-layer refusal reads differently from a runtime
//! failure. The model's JSON path is unchanged — the full error still
//! reaches it.

use super::super::tool_policy::{
    ApprovalState, ExecutionDecision, PolicyDenial, execution_decision, external_side_effect_gated,
};
use super::*;
use crate::{AgentLimits, LocalStateEffect, ToolDefinition, ToolEffect, ToolError, ToolExecutor};

/// The fetch-shaped declaration: an external side effect approved once by
/// the run's scope, not per call — the exact combination the misconfiguration
/// guard exists to refuse.
fn plan_gated_egress() -> ToolDefinition {
    ToolDefinition {
        name: "http_fetch".into(),
        description: "the fetch lane".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: crate::ToolConcurrency::Serial,
        completion: None,
    }
}

/// Default limits deny the plan-gated egress tool in both paths: the guard
/// is unchanged wherever the permit is false, so interactive `ask` turns are
/// byte-identical, and an author who set the bit carelessly is still refused.
#[test]
fn the_guard_stands_everywhere_the_permit_is_false() {
    let definition = plan_gated_egress();
    assert!(
        external_side_effect_gated(&definition, &AgentLimits::default()),
        "the misconfiguration guard keeps denying by default"
    );
    assert!(
        matches!(
            execution_decision(
                &definition,
                &AgentLimits::default(),
                ApprovalState::NotRequired,
            ),
            ExecutionDecision::Deny(PolicyDenial::ExternalSideEffect)
        ),
        "the batch path refuses it by default"
    );
}

/// The permit opens the gated tool in the shared execution policy, so a
/// permit granted per step cannot run in one path and not the other.
#[test]
fn the_permit_opens_the_gated_tool_in_both_paths() {
    let definition = plan_gated_egress();
    let limits = AgentLimits {
        permit_external_effects: true,
        ..AgentLimits::default()
    };
    assert!(
        !external_side_effect_gated(&definition, &limits),
        "the plan-gated egress permit satisfies the guard"
    );
    assert!(
        matches!(
            execution_decision(&definition, &limits, ApprovalState::NotRequired),
            ExecutionDecision::Allow
        ),
        "with the permit, the shared policy allows the tool"
    );
}

/// The permit cannot smuggle a write-shaped tool past the other gates: the
/// fetch downloader is external *and* a workspace write, so it stays denied
/// by the write permit even with external effects permitted.
#[test]
fn the_permit_does_not_carry_the_write_shaped_members() {
    let mut definition = plan_gated_egress();
    definition.name = "http_download".into();
    definition.effect.local_state = LocalStateEffect::WriteWorkspace;
    let limits = AgentLimits {
        permit_external_effects: true,
        ..AgentLimits::default()
    };
    assert!(
        !external_side_effect_gated(&definition, &limits),
        "the external gate is satisfied by the permit"
    );
    assert!(
        matches!(
            execution_decision(&definition, &limits, ApprovalState::NotRequired),
            ExecutionDecision::Deny(PolicyDenial::WorkspaceWrite)
        ),
        "the workspace-write gate still refuses it: one permit per shape"
    );
}

/// The drift pin: the exported cap is exactly the number the loop's own
/// `tool_message` truncates at. A serialized result at the cap arrives whole;
/// one byte over it is cut with the marker. The harness's fetch bound
/// derives from this same helper, so the pre-bound and the truncation point
/// are the same number by construction.
#[test]
fn the_exported_cap_is_the_loop_s_truncation_point() {
    let budget = usize::MAX;
    let cap = tool_message_cap(budget);
    assert_eq!(
        cap, MAX_TOOL_MESSAGE_BYTES,
        "the absolute ceiling is the cap for any larger budget"
    );
    // A string whose serialized form is exactly the cap arrives untruncated.
    let whole = Value::String("x".repeat(cap - 2));
    assert_eq!(
        serde_json::to_string(&whole).unwrap().len(),
        cap,
        "test setup: the value serializes to exactly the cap"
    );
    let (message, truncated) = tool_message("c1".into(), whole, budget);
    assert!(
        !truncated && message.content.len() == cap,
        "a result at the cap is whole: {truncated}, {}",
        message.content.len()
    );
    // One byte over the cap is cut, with the loop's own marker.
    let over = Value::String("x".repeat(cap - 1));
    let (message, truncated) = tool_message("c1".into(), over, budget);
    assert!(
        truncated && message.content.len() <= cap,
        "a result over the cap is cut at the cap: {truncated}, {}",
        message.content.len()
    );
    assert!(
        message.content.contains("[truncated]"),
        "the cut must be visible to the model: {}",
        message.content.len()
    );
    // A budget tighter than the absolute ceiling binds first — the helper is
    // `min`, both ways.
    assert_eq!(tool_message_cap(1024), 1024);
}

/// A failing executor returning one fixed error, so `execute`'s failure
/// summary can be asserted without a provider or a run.
struct FailingExecutor {
    error: ToolError,
}

#[async_trait::async_trait]
impl ToolExecutor for FailingExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Err(self.error.clone())
    }
}

/// An executor that always succeeds — the unchanged-success-path pin.
struct OkUnitExecutor;

#[async_trait::async_trait]
impl ToolExecutor for OkUnitExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({"rows": 1}))
    }
}

fn read_only_definition() -> ToolDefinition {
    ToolDefinition {
        name: "bounded_sql_query".into(),
        description: "read-only query".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: crate::ToolConcurrency::Serial,
        completion: None,
    }
}

fn failing_error() -> ToolError {
    ToolError::QueryFailedDetail(
        "query rejected by read-only safety policy: DROP modifies data or schema".into(),
    )
}

fn runtime_error() -> ToolError {
    ToolError::QueryFailedDetail("connection reset by peer".into())
}

async fn run_failed(executor: &dyn ToolExecutor, sql: &str) -> (serde_json::Value, String) {
    let definition = read_only_definition();
    execute(
        executor,
        &definition.name.clone(),
        serde_json::json!({"sql": sql}),
        Some(&definition),
    )
    .await
}

/// A refused write names the safety rejection in the human-facing summary:
/// the reason that already reaches the model must also reach the user.
#[tokio::test]
async fn a_refused_write_says_it_was_refused() {
    let executor = FailingExecutor {
        error: failing_error(),
    };
    let (_, summary) = run_failed(&executor, "DROP TABLE t").await;
    assert!(
        summary.contains("failed"),
        "the summary keeps the failure signal: {summary:?}"
    );
    assert!(
        summary.contains("query rejected by read-only safety policy"),
        "the refusal reason must reach the human summary: {summary:?}"
    );
}

/// Two failures with different causes produce different lines — the packet's
/// gate: a refusal must not render identically to a runtime failure.
#[tokio::test]
async fn a_runtime_failure_reads_differently_from_a_refusal() {
    let refused = run_failed(
        &FailingExecutor {
            error: failing_error(),
        },
        "DROP TABLE t",
    )
    .await
    .1;
    let runtime = run_failed(
        &FailingExecutor {
            error: runtime_error(),
        },
        "SELECT 1",
    )
    .await
    .1;
    assert_ne!(
        refused, runtime,
        "refusal and runtime failure must render differently"
    );
    assert!(
        runtime.contains("connection reset by peer"),
        "the runtime line names its own reason: {runtime:?}"
    );
}

/// An oversized error is truncated to the documented cap: an unbounded
/// connector error must not flood the transcript or a persisted session.
#[tokio::test]
async fn a_long_error_is_bounded() {
    let long = "x".repeat(MAX_FAILURE_REASON_CHARS + 500);
    let executor = FailingExecutor {
        error: ToolError::QueryFailedDetail(long),
    };
    let (_, summary) = run_failed(&executor, "SELECT 1").await;
    let reason = summary
        .strip_prefix("read-only database tool failed — ")
        .expect("the failure keeps its generic prefix: {summary:?}");
    assert!(
        reason.chars().count() <= MAX_FAILURE_REASON_CHARS + 1,
        "the reason must be bounded: chars={}",
        reason.chars().count()
    );
    assert!(
        summary.ends_with('…'),
        "bounded reasons use the existing … convention: {summary:?}"
    );
}

/// The model's JSON path is unchanged: it carries the full, untruncated
/// error even when the human summary is bounded. This test stops the packet
/// trading the model's context for the user's.
#[tokio::test]
async fn the_model_still_receives_the_full_error() {
    let full = failing_error().to_string();
    let executor = FailingExecutor {
        error: failing_error(),
    };
    let (value, _) = run_failed(&executor, "DROP TABLE t").await;
    assert_eq!(
        value,
        serde_json::json!({"error": full}),
        "the model JSON must carry the full error text unchanged"
    );
    let long = "y".repeat(MAX_FAILURE_REASON_CHARS + 500);
    let executor = FailingExecutor {
        error: ToolError::QueryFailedDetail(long.clone()),
    };
    let (value, _) = run_failed(&executor, "SELECT 1").await;
    assert_eq!(
        value,
        serde_json::json!({"error": format!("read-only query failed: {long}")}),
        "even an oversized error reaches the model whole; only the summary is cut"
    );
}

/// The refusal line explains what was refused; it must not offer an
/// override, suggest a bypass, or imply the write could be retried as one.
#[tokio::test]
async fn a_refusal_offers_no_override() {
    let executor = FailingExecutor {
        error: failing_error(),
    };
    let (_, summary) = run_failed(&executor, "DROP TABLE t").await;
    for word in ["force", "bypass", "override", "--allow", "disable"] {
        assert!(
            !summary.contains(word),
            "the refusal must not suggest {word:?}: {summary:?}"
        );
    }
}

/// The success path is untouched: `execute` on `Ok` still reports the
/// declared completion with no reason suffix.
#[tokio::test]
async fn a_successful_tool_summary_is_unchanged() {
    let definition = read_only_definition();
    let (value, summary) = execute(
        &OkUnitExecutor,
        &definition.name.clone(),
        serde_json::json!({"sql": "SELECT 1"}),
        Some(&definition),
    )
    .await;
    assert_eq!(value, serde_json::json!({"rows": 1}));
    assert_eq!(
        summary, "read-only database tool completed",
        "success wording is byte-exact: {summary:?}"
    );
}

// -- the redaction announcement (D8 follow-up): the model must be able to --
// -- tell a masked value from corruption, and must never write the --------
// -- placeholder back over the real one. -----------------------------------

/// A tool result whose content is secret-shaped reaches the model with the
/// value masked *and* a fixed note naming how many replacements were made,
/// so the model can tell masking from corruption instead of "fixing" a
/// `[redacted]` it reads back. Two secrets in the same result count 2, not
/// one note per secret.
#[test]
fn a_redacted_tool_result_tells_the_model() {
    let one = serde_json::json!({"value": "token=abc123"});
    let (message, _) = tool_message("c1".into(), one, usize::MAX);
    assert!(
        message.content.contains("[redacted]"),
        "the secret must still be masked: {}",
        message.content
    );
    assert!(
        message
            .content
            .contains("[saya: 1 secret-shaped value(s) in this result were replaced with [redacted]; the source is unchanged — do not write [redacted] back]"),
        "the note must name the exact count: {}",
        message.content
    );

    let two = serde_json::json!({"value": "token=abc123 password=xyz"});
    let (message, _) = tool_message("c2".into(), two, usize::MAX);
    assert!(
        message
            .content
            .contains("[saya: 2 secret-shaped value(s) in this result were replaced with [redacted]; the source is unchanged — do not write [redacted] back]"),
        "two secrets must count 2: {}",
        message.content
    );
}

/// Nothing secret-shaped in the result: the content is byte-identical to the
/// un-noted redaction — no note is ever appended for a clean result.
#[test]
fn an_unredacted_tool_result_is_unchanged() {
    let result = serde_json::json!({"value": "SELECT 1 WHERE status='done'"});
    let (message, _) = tool_message("c1".into(), result.clone(), usize::MAX);
    let expected = serde_json::to_string(&result).unwrap();
    assert_eq!(
        message.content, expected,
        "a clean result must be byte-identical, with no note prepended: {}",
        message.content
    );
    assert!(
        !message.content.contains("[saya:"),
        "no note may appear when nothing was redacted: {}",
        message.content
    );
}

// -- the shaping extraction (R3): the capture hook must shape the model's --
// -- view with the exact function the loop uses, so both see the same ------
// -- bytes. The pin: the extracted function reproduces the pre-extraction --
// -- `tool_message` body byte for byte. ------------------------------------

/// The pre-extraction `tool_message` shaping, embedded as the reference:
/// serialize, cut to the cap on a char boundary with the visible marker, then
/// redact what remains and prepend the note when anything was replaced. Only
/// `redact_counted` and `tool_message_cap` — unchanged external contracts,
/// pinned by their own tests — are called from production; everything else
/// here is the literal old code, so the comparison cannot drift with it.
fn old_tool_message_shaping(result: &Value, byte_budget: usize) -> (String, bool) {
    let cap = tool_message_cap(byte_budget);
    let text = serde_json::to_string(result)
        .unwrap_or_else(|_| "{\"error\":\"tool result unavailable\"}".into());
    let (content, truncated) = if text.len() <= cap {
        (text, false)
    } else {
        let marker = "…[truncated: tool result exceeded the conversation byte budget]";
        let head = cap.saturating_sub(marker.len());
        let mut idx = head;
        if idx >= text.len() {
            idx = text.len();
        }
        while idx > 0 && !text.is_char_boundary(idx) {
            idx -= 1;
        }
        let mut truncated = String::from(&text[..idx]);
        truncated.push_str(marker);
        (truncated, true)
    };
    let (content, redacted_count) = redact_counted(&content);
    let content = if redacted_count > 0 {
        format!(
            "[saya: {redacted_count} secret-shaped value(s) in this result were replaced with \
             [redacted]; the source is unchanged — do not write [redacted] back]\n\n{content}"
        )
    } else {
        content
    };
    (content, truncated)
}

/// The extraction pin (R3): `shape_tool_result` returns exactly the bytes the
/// loop's `tool_message` has always put in the model's context — across a
/// clean result, a redacted one (note prepended, count named), whole-at-the-
/// cap, one-byte-over, a tight budget cutting on a multi-byte boundary, and a
/// secret that survives into the truncated head (redaction after the cut).
/// `tool_message` must shape through the same function — no second
/// implementation to drift.
#[test]
fn shaping_matches_tool_message_and_caps_cut_results() {
    let cap = tool_message_cap(usize::MAX);
    let cases: Vec<(Value, usize, usize)> = vec![
        // A clean, small result — the common case: whole, no redactions.
        (
            serde_json::json!({"columns": ["id"], "rows": [[1], [2]], "row_count": 2}),
            usize::MAX,
            0,
        ),
        // Two secret-shaped values: note prepended, count 2.
        (
            serde_json::json!({"value": "token=abc123 password=xyz"}),
            usize::MAX,
            2,
        ),
        // Exactly at the cap: whole.
        (Value::String("x".repeat(cap - 2)), usize::MAX, 0),
        // One byte over the cap: cut with the loop's own marker.
        (Value::String("x".repeat(cap - 1)), usize::MAX, 0),
        // A tight budget cuts inside a multi-byte sequence: the boundary is
        // floored, never split.
        (serde_json::json!({"text": "é".repeat(64)}), 64, 0),
        // A tight budget whose truncated head still carries a secret: the
        // redaction runs on the CUT text and the note prepends — order pinned.
        (
            serde_json::json!({"text": format!("token=abc123 {}", "y".repeat(200))}),
            120,
            1,
        ),
        // Trivial shapes stay trivial.
        (serde_json::json!({}), usize::MAX, 0),
        (Value::Null, usize::MAX, 0),
    ];
    for (value, budget, redactions) in cases {
        let shaped = shape_tool_result(&value, budget);
        if !shaped.truncated {
            let (old_content, old_truncated) = old_tool_message_shaping(&value, budget);
            assert_eq!(shaped.text.as_bytes(), old_content.as_bytes());
            assert_eq!(shaped.truncated, old_truncated);
        }
        assert!(
            shaped.text.len() <= tool_message_cap(budget),
            "a cut result must remain under the cap (budget {budget})"
        );
        if !shaped.truncated {
            assert_eq!(
                shaped.redactions, redactions,
                "the redaction count must match the old body (budget {budget})"
            );
        }
        // `tool_message` has no second implementation: its content is exactly
        // the extracted shaping's text.
        let (message, truncated) = tool_message("c1".into(), value.clone(), budget);
        assert_eq!(
            message.content, shaped.text,
            "tool_message must shape through the one function (budget {budget})"
        );
        assert_eq!(truncated, shaped.truncated);
    }
}

/// The final model-facing text, not merely its serialized prefix, must fit its
/// cap. A marker, redaction replacement, or redaction notice cannot turn a
/// bounded result into an oversized provider message.
#[test]
fn shaping_never_exceeds_its_cap_for_tiny_unicode_and_redacted_results() {
    for (value, budget) in [
        (serde_json::json!({"text": "é".repeat(64)}), 1),
        (serde_json::json!({"token": "token=abc123"}), 8),
        (serde_json::json!({"text": "x".repeat(256)}), 16),
    ] {
        let shaped = shape_tool_result(&value, budget);
        assert!(
            shaped.text.len() <= tool_message_cap(budget),
            "final shaped text exceeds {} bytes: {}",
            tool_message_cap(budget),
            shaped.text.len()
        );
        assert!(
            std::str::from_utf8(shaped.text.as_bytes()).is_ok(),
            "shaped output must remain UTF-8"
        );
    }
}

/// A serializer may offer an entire JSON string in one write. The bounded
/// writer still preserves the useful prefix of that one write, rather than
/// keeping only structural JSON fragments before the truncation marker.
#[test]
fn a_single_large_unicode_write_keeps_a_visible_prefix() {
    let shaped = shape_tool_result(&Value::String("星".repeat(256)), 64);
    assert!(shaped.truncated, "the oversized value must be marked cut");
    assert!(
        shaped.text.contains('星'),
        "a useful Unicode prefix survives"
    );
    assert!(shaped.text.contains("[truncated]"), "the cut stays visible");
    assert!(
        shaped.text.len() <= 64,
        "the final text stays within the cap"
    );
}

/// A tool-turn budget is divided before execution, so every accepted call has
/// a deterministic share and their retained model-facing contents cannot add
/// up past the turn's aggregate allowance.
#[test]
fn turn_result_caps_share_the_aggregate_budget_without_a_minimum() {
    assert_eq!(turn_result_caps(10, 3), vec![4, 3, 3]);
    assert_eq!(turn_result_caps(2, 3), vec![1, 1, 0]);
    let caps = turn_result_caps(usize::MAX, 3);
    assert_eq!(caps, vec![MAX_TOOL_MESSAGE_BYTES; 3]);
    assert!(caps.iter().sum::<usize>() <= 256 * 1024);
}
