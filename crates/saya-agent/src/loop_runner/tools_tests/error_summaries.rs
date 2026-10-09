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
