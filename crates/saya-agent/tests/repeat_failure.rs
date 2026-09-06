//! Loop invariant: a statement that already failed in this run is not
//! re-executed, and a run that exhausts its budget surfaces its best available
//! answer. These drive `run_agent_with_sink` end-to-end with a canned provider
//! and a recording executor, asserting the observable contract: execution
//! counts, the refusal message the model receives, and the nominated SQL on
//! salvage.
//!
//! The cap on remembered failures is pinned by an inline unit test in
//! `loop_runner/failed_statements.rs` (it references the private
//! `MAX_REMEMBERED_FAILURES` constant); this file covers the end-to-end
//! behaviours.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatMessage,
    ChatProvider, ChatRequest, ChatResponse, LocalStateEffect, ProviderError, ToolCall,
    ToolDefinition, ToolEffect, ToolError, ToolExecutor, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

/// A provider that returns one canned assistant message per turn (in order)
/// and records every request it was sent, so a test can inspect the tool
/// messages the model received. The default `stream` delegates to `complete`,
/// which the loop drives once per turn.
struct CannedProvider {
    responses: Mutex<Vec<ChatMessage>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait]
impl ChatProvider for CannedProvider {
    fn name(&self) -> &str {
        "canned"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let message = self.responses.lock().unwrap().remove(0);
        Ok(ChatResponse::new(message))
    }
}

/// An executor that records each call's name and either always fails (with a
/// recognisable error) or always succeeds. The recorded length is the
/// execution count — the core assertion for repeat refusal.
struct ScriptedExecutor {
    calls: Arc<Mutex<Vec<String>>>,
    fail: bool,
    error: String,
}

#[async_trait]
impl ToolExecutor for ScriptedExecutor {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        if self.fail {
            Err(ToolError::QueryFailedDetail(self.error.clone()))
        } else {
            Ok(serde_json::json!({"rows": 1}))
        }
    }
}

struct RecordingSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// A read-only database query tool carrying a `sql` string argument. It needs
/// no approval so single-call turns run on the sequential path and multi-call
/// turns run on the batch path — both exercise the repeat-refusal and outcome
/// recording logic.
fn query_tool() -> ToolDefinition {
    ToolDefinition {
        name: "bounded_sql_query".into(),
        description: "read-only query".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
    }
}

fn call(id: &str, sql: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "bounded_sql_query".into(),
        arguments: serde_json::json!({"sql": sql}),
    }
}

fn tool_turn(id: &str, sql: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![call(id, sql)],
        tool_call_id: None,
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "answer the question".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// Runs the agent with a canned provider returning the given responses and a
/// scripted executor, returning `(output, calls, requests)`.
async fn run(
    responses: Vec<ChatMessage>,
    executor: ScriptedExecutor,
    limits: AgentLimits,
) -> (saya_agent::AgentOutput, Vec<String>, Vec<ChatRequest>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CannedProvider {
        responses: Mutex::new(responses),
        requests: requests.clone(),
    };
    let output = run_agent_with_sink(
        &provider,
        &executor,
        request(),
        vec![query_tool()],
        limits,
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: Arc::new(Mutex::new(Vec::new())),
        },
        saya_agent::CancellationToken::new(),
    )
    .await
    .expect("run completes");
    let calls = executor.calls.lock().unwrap().clone();
    let reqs = requests.lock().unwrap().clone();
    (output, calls, reqs)
}

/// The most recent `tool`-role message whose content contains `needle`, from
/// the messages sent on the last provider turn — i.e. what the model actually
/// received for the failing/repeated call.
fn last_tool_message_containing(requests: &[ChatRequest], needle: &str) -> Option<String> {
    requests.last().and_then(|req| {
        req.messages
            .iter()
            .rev()
            .find(|m| m.role == "tool" && m.content.contains(needle))
            .map(|m| m.content.clone())
    })
}

/// A statement that failed once is not executed a second time: the second
/// submission is refused, the executor runs once, and the refusal the model
/// receives names the error the first attempt produced. Against the code before
/// the fix the executor runs twice (the loop re-dispatched the identical
/// failing statement every turn).
#[tokio::test]
async fn a_failed_statement_is_not_re_executed_and_the_refusal_names_the_error() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "syntax error near 'bad'".into(),
    };
    let (output, calls, requests) = run(
        vec![
            tool_turn("c1", "SELECT bad"),
            tool_turn("c2", "SELECT bad"),
            ChatMessage::text("assistant", "changed approach"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(
        output.answer, "changed approach",
        "the run completes after the refusal"
    );
    assert_eq!(
        calls.len(),
        1,
        "the identical failing statement must execute once, not twice: {calls:?}"
    );
    let refusal = last_tool_message_containing(&requests, "already failed")
        .expect("the refusal must reach the model as a tool message");
    assert!(
        refusal.contains("syntax error near 'bad'"),
        "the refusal must name the earlier error, got: {refusal}"
    );
}

/// A statement that differs by one character is NOT a repeat — there is no
/// whitespace or case normalisation, so a genuine edit is always executed.
/// Against a normalising implementation this would execute once.
#[tokio::test]
async fn a_statement_differing_by_one_character_is_executed() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "bad".into(),
    };
    let (_output, calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT 1"),
            tool_turn("c2", "SELECT 2"),
            ChatMessage::text("assistant", "done"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(
        calls.len(),
        2,
        "a one-character edit is a different statement and must execute: {calls:?}"
    );
}

/// A statement that succeeded is not tracked — re-running a successful query
/// is legitimate. The same SQL submitted twice, both succeeding, executes
/// twice.
#[tokio::test]
async fn a_successful_statement_may_be_re_run() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: false,
        error: String::new(),
    };
    let (_output, calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT 1"),
            tool_turn("c2", "SELECT 1"),
            ChatMessage::text("assistant", "done"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(
        calls.len(),
        2,
        "a successful statement may be re-run: {calls:?}"
    );
}

/// A normal run that never repeats a failure executes every call exactly once
/// — the repeat-refusal logic must not suppress a first execution. Three
/// distinct statements, none repeated, all execute.
#[tokio::test]
async fn a_normal_run_never_repeating_a_failure_executes_every_call_once() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: false,
        error: String::new(),
    };
    let (output, calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT 1"),
            tool_turn("c2", "SELECT 2"),
            tool_turn("c3", "SELECT 3"),
            ChatMessage::text("assistant", "done"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(output.answer, "done");
    assert_eq!(
        calls.len(),
        3,
        "distinct statements must all execute once, none suppressed: {calls:?}"
    );
}

/// A run that exhausts its turn budget with a successful query and no
/// nomination ends with that query nominated. The model never called
/// `designate_answer` (the budget ran out first), so the runner surfaces the
/// last statement that completed successfully. Against the code before the
/// fix `answer_sql` was `None` even though a query succeeded.
#[tokio::test]
async fn budget_exhaustion_with_a_successful_query_nominates_it() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: false,
        error: String::new(),
    };
    let (output, _calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT count(*) FROM t"),
            ChatMessage::text("assistant", "best effort from gathered results"),
        ],
        executor,
        AgentLimits {
            max_turns: Some(1),
            ..AgentLimits::default()
        },
    )
    .await;
    assert!(
        output.truncated,
        "a budget-exhausted run must be marked truncated"
    );
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the last successful statement is nominated when the model never did"
    );
}

/// A run whose only statements failed nominates nothing — a wrong nomination
/// is worse than an absent one. Only successful statements are nomination
/// candidates, so an all-failure run leaves `answer_sql` absent.
#[tokio::test]
async fn budget_exhaustion_with_only_failed_statements_nominates_nothing() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "bad".into(),
    };
    let (output, _calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT bad"),
            ChatMessage::text("assistant", "best effort"),
        ],
        executor,
        AgentLimits {
            max_turns: Some(1),
            ..AgentLimits::default()
        },
    )
    .await;
    assert!(
        output.truncated,
        "a budget-exhausted run must be marked truncated"
    );
    assert!(
        output.answer_sql.is_none(),
        "an all-failure run must nominate nothing, got {:?}",
        output.answer_sql
    );
}

/// The batch path records failures too: two distinct statements that fail
/// concurrently in one turn are both remembered, so a later single-turn repeat
/// of either is refused on the sequential path (the batch is not taken when a
/// repeat is present). This proves the two execution paths share the same
/// failure memory.
#[tokio::test]
async fn the_batch_path_records_failures_so_a_later_repeat_is_refused() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "bad batch".into(),
    };
    let (output, calls, requests) = run(
        vec![
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![call("c1", "SELECT a"), call("c2", "SELECT b")],
                tool_call_id: None,
            },
            tool_turn("c3", "SELECT a"),
            ChatMessage::text("assistant", "done"),
        ],
        executor,
        AgentLimits::default(),
    )
    .await;
    assert_eq!(output.answer, "done");
    assert_eq!(
        calls.len(),
        2,
        "the two distinct batch calls execute; the later repeat is refused: {calls:?}"
    );
    let refusal = last_tool_message_containing(&requests, "already failed")
        .expect("the repeat refusal must reach the model");
    assert!(
        refusal.contains("bad batch"),
        "the refusal must name the earlier batch error, got: {refusal}"
    );
}
