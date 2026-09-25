//! The side-effect guard and the tool-message cap: the S2 loop seam's pins.
//!
//! Phase 7 packet 2: a failed tool's human-facing summary names *why* it
//! failed, so a safety-layer refusal reads differently from a runtime
//! failure. The model's JSON path is unchanged — the full error still
//! reaches it.

use super::*;
use crate::{LocalStateEffect, ToolDefinition, ToolEffect, ToolError, ToolExecutor};

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
        !auto_runnable(&definition, &AgentLimits::default()),
        "the batch path refuses it by default"
    );
}

/// The permit opens the gated tool in the shared policy both paths consult:
/// `auto_runnable` (the batch path) and the sequential path's by-name gate
/// resolve through the same function, so a permit granted per step cannot
/// run in one path and not the other.
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
        auto_runnable(&definition, &limits),
        "with the permit, a no-approval external tool is auto-runnable in \
         the batch path — and the sequential path consults the same gate"
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
        !auto_runnable(&definition, &limits),
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
        message.content.contains("[truncated:"),
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
