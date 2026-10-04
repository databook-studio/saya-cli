//! Tests for the ask path's clarification outcome (B3c): a scripted provider
//! that stops to ask must surface on both surfaces — the headless exit code
//! (6, the paused class) and the TUI transcript's Question block.

use super::ask_exit_code;
use super::settle_ask;
use crate::agent::runtime::AgentRuntimeError;
use crate::agent::tools::DatabaseTools;
use crate::commands::{capture_output_start, capture_output_take};
use crate::render::{RenderFormat, clarification_text, render_event};
use crate::stream_render::terminal_event;
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentOutput, AgentRequest, AllowReadOnlyApproval, BudgetDimension,
    CancellationReason, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    ProviderError, ProviderStream, TokenUsage, ToolCall, run_agent_with_sink,
};
use std::sync::Mutex;

/// The scripted provider: one turn — the model asks which metric "active
/// users" means instead of assuming one. A second provider call would pop
/// from an empty vec and panic, so "the turn ended" is proven by the run
/// returning at all.
struct ScriptedProvider {
    turns: Mutex<Vec<ChatMessage>>,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        panic!("the loop must drive providers through stream()")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let message = self.turns.lock().unwrap().remove(0);
        let events = if message.tool_calls.is_empty() {
            vec![
                Ok(saya_agent::ProviderEvent::TextDelta(message.content)),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(message.tool_calls)),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// The question the scripted model asks — deliberately a metric ambiguity,
/// the material definition the guidance names.
const QUESTION: &str = "Which definition of \"active users\" should I use?";
const OPTIONS: [&str; 2] = [
    "sessions in the last 30 days",
    "purchases in the last 90 days",
];

fn clarification_turn() -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![ToolCall {
            id: "call-1".into(),
            name: saya_agent::REQUEST_CLARIFICATION_TOOL.into(),
            arguments: serde_json::json!({
                "question": QUESTION,
                "options": OPTIONS,
            }),
        }],
        tool_call_id: None,
    }
}

fn agent_request() -> AgentRequest {
    AgentRequest {
        prompt: "how many active users do we have".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// One scripted run, both surfaces observed: the headless mapping exits 6,
/// the TUI block carries the same question, and the structured event rides
/// the NDJSON stream.
#[tokio::test]
async fn ambiguous_metric_requires_input_across_surfaces() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![clarification_turn()]),
    };
    // The real definitions (the tool advertised) and a no-connector executor:
    // the ask needs no database.
    let definitions = DatabaseTools::definitions(false, false, false, false, false);
    let executor = DatabaseTools::new(None, 10, false);
    let output = run_agent_with_sink(
        &provider,
        &executor,
        agent_request(),
        definitions,
        saya_agent::AgentLimits::default(),
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("the turn ends on the clarification, never in an error");

    // Headless surface: the run ended on a clarification, so `saya ask`
    // returns the paused exit code, not 0 and not a failure.
    assert_eq!(
        ask_exit_code(&output),
        6,
        "the paused-for-input class is exit 6: {:?}",
        output.events
    );

    // The TUI surface: the transcript's Question block carries the same
    // question. The tui subtree's `apply_event` is private to it, so the
    // block is observed through the shared shaper both adapters render from —
    // the stream_events tests pin that `apply_event` pushes exactly this text.
    let options: Vec<String> = OPTIONS.iter().map(|option| option.to_string()).collect();
    let block_text = clarification_text(QUESTION, &options);
    assert!(
        block_text.contains(QUESTION),
        "the TUI block carries the question: {block_text:?}"
    );
    assert!(
        block_text.contains("1. sessions in the last 30 days")
            && block_text.contains("2. purchases in the last 90 days"),
        "the TUI block numbers the options: {block_text:?}"
    );

    // And the headless NDJSON stream carries the structured event.
    let event = output
        .events
        .iter()
        .find(|event| matches!(event, AgentEvent::ClarificationNeeded { .. }))
        .expect("the clarification event is on the run");
    let terminal = terminal_event(event.clone()).expect("the event renders headlessly");
    let rendered = render_event(&terminal, RenderFormat::Ndjson);
    assert!(
        rendered
            .stdout
            .contains(r#""event":"clarification_needed""#),
        "the ndjson stream tags it: {:?}",
        rendered.stdout
    );
}

/// A run that never asked exits 0 — the mapping must not leak the paused
/// class into ordinary completions.
#[tokio::test]
async fn an_ordinary_completion_exits_zero() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![ChatMessage::text("assistant", "42 active users")]),
    };
    let definitions = DatabaseTools::definitions(false, false, false, false, false);
    let executor = DatabaseTools::new(None, 10, false);
    let output = run_agent_with_sink(
        &provider,
        &executor,
        agent_request(),
        definitions,
        saya_agent::AgentLimits::default(),
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("an ordinary turn completes");
    assert_eq!(ask_exit_code(&output), 0);
    assert_eq!(output.answer, "42 active users");
}

// ===========================================================================
// The one classification for a settled `ask`. After the work has settled —
// on the work-ready branch or the setup-error branch, and independent of
// which select branch won — the question token's first stop reason decides:
// User 130, Budget 6 with truthful partial wording, OwnerFailure 5, and only
// a run that stopped for no typed reason falls through to the old mapping
// (clarification 6, natural 0, setup error 5).
// ===========================================================================

/// A natural completion: the model answered, nothing stopped.
fn natural_output() -> AgentOutput {
    AgentOutput {
        answer: "42 active users".into(),
        events: Vec::new(),
        used_bounded_sql_query: true,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

/// The same completion whose run paused for input instead of answering.
fn clarification_output() -> AgentOutput {
    let mut out = natural_output();
    out.events.push(AgentEvent::clarification_needed(
        QUESTION,
        OPTIONS.iter().map(|option| option.to_string()).collect(),
    ));
    out
}

/// A Budget(ElapsedTime)-latched question that still produced a natural
/// output (the deadline expired at/after verification) exits 6 — the
/// partial/stopped class — and the printed diagnostic says the answer is
/// partial. Before the repair this returned 0 with no wording.
#[test]
fn a_budget_latched_question_with_a_natural_answer_exits_six_with_partial_wording() {
    let cancellation = CancellationToken::new();
    cancellation.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
    capture_output_start();
    let code = settle_ask(Ok(natural_output()), &cancellation, RenderFormat::Ndjson)
        .expect("a settled ask maps to an exit code, not an error");
    let (stdout, stderr) = capture_output_take();
    assert_eq!(
        code, 6,
        "the budget-stopped class is the paused exit: {code}"
    );
    let printed = format!("{stdout}{stderr}");
    assert!(
        printed.contains("partial"),
        "the diagnostic says the answer is partial/stopped: {printed:?}"
    );
}

/// A User-latched token on the work-ready branch exits 130 — the same class
/// the ready-signal branch returns — so a ready-work/ready-signal race
/// classifies the same either way. Before the repair this returned 0.
#[test]
fn a_user_latched_question_exits_130_on_the_work_ready_branch() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let code = settle_ask(Ok(natural_output()), &cancellation, RenderFormat::Ndjson)
        .expect("a settled ask maps to an exit code");
    assert_eq!(
        code, 130,
        "a user-stopped run is the interrupted class: {code}"
    );
}

/// A budget stop before any answer existed (setup error) still classifies
/// from the token: exit 6, without inventing output — and the wording stays
/// truthful that nothing finished.
#[test]
fn a_budget_latched_setup_failure_exits_six_without_inventing_an_answer() {
    let cancellation = CancellationToken::new();
    cancellation.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
    capture_output_start();
    let code = settle_ask(
        Err(AgentRuntimeError::Agent("request cancelled".into())),
        &cancellation,
        RenderFormat::Ndjson,
    )
    .expect("a settled ask maps to an exit code");
    let (stdout, stderr) = capture_output_take();
    assert_eq!(
        code, 6,
        "the budget-stopped class on the error branch: {code}"
    );
    let printed = format!("{stdout}{stderr}");
    assert!(
        printed.contains("partial"),
        "the diagnostic stays truthful about the unfinished answer: {printed:?}"
    );
}

/// A user stop before any answer existed exits 130 too.
#[test]
fn a_user_latched_setup_failure_exits_130() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let code = settle_ask(
        Err(AgentRuntimeError::Agent("request cancelled".into())),
        &cancellation,
        RenderFormat::Ndjson,
    )
    .expect("a settled ask maps to an exit code");
    assert_eq!(
        code, 130,
        "a user-stopped setup is the interrupted class: {code}"
    );
}

/// An OwnerFailure stop on the work-ready branch exits 5 with a message that
/// says what actually stopped the run.
#[test]
fn an_owner_failure_on_the_work_ready_branch_exits_five_with_a_truthful_message() {
    let cancellation = CancellationToken::new();
    cancellation.cancel_with_reason(CancellationReason::OwnerFailure);
    capture_output_start();
    let code = settle_ask(Ok(natural_output()), &cancellation, RenderFormat::Ndjson)
        .expect("a settled ask maps to an exit code");
    let (stdout, stderr) = capture_output_take();
    assert_eq!(code, 5, "an owner failure is the failure class: {code}");
    let printed = format!("{stdout}{stderr}");
    assert!(
        printed.contains("owner failure"),
        "the diagnostic names the owner failure: {printed:?}"
    );
}

/// An OwnerFailure stop on the setup-error branch keeps exit 5 and keeps the
/// run's own error as the message — the truth the owner failure interrupted.
#[test]
fn an_owner_failure_on_a_setup_error_keeps_exit_five_with_the_run_s_own_error() {
    let cancellation = CancellationToken::new();
    cancellation.cancel_with_reason(CancellationReason::OwnerFailure);
    capture_output_start();
    let code = settle_ask(
        Err(AgentRuntimeError::Provider("provider down".into())),
        &cancellation,
        RenderFormat::Ndjson,
    )
    .expect("a settled ask maps to an exit code");
    let (stdout, stderr) = capture_output_take();
    assert_eq!(code, 5);
    let printed = format!("{stdout}{stderr}");
    assert!(
        printed.contains("provider down"),
        "the run's own error is the message: {printed:?}"
    );
}

/// A question that stopped for no typed reason keeps the existing exits:
/// a natural answer 0, a settled clarification 6.
#[test]
fn a_question_that_stopped_for_no_reason_keeps_the_existing_exit() {
    let cancellation = CancellationToken::new();
    let natural = settle_ask(Ok(natural_output()), &cancellation, RenderFormat::Ndjson)
        .expect("a settled ask maps to an exit code");
    assert_eq!(natural, 0, "an ordinary completion still exits 0");
    let paused = settle_ask(
        Ok(clarification_output()),
        &cancellation,
        RenderFormat::Ndjson,
    )
    .expect("a settled ask maps to an exit code");
    assert_eq!(paused, 6, "a settled clarification still exits 6");
}
