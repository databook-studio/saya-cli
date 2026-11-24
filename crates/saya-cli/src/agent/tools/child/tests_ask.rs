//! The coding surfaces' clarification ask: the ask is a database tool, so it
//! is advertised exactly where the job selected a profile AND an operator
//! surface can answer it, and a call ends the turn — the model is told the
//! user's next message is the answer.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentLimits, AgentRequest, CancellationToken, ChatMessage, ChatProvider,
    ChatRequest, ChatResponse, NoopEventSink, ProviderError, ProviderStream, ToolCall,
    ToolExecutor, run_agent_with_sink,
};
use saya_harness::coding::policy::ChildAuthorityView;
use saya_types::ManagedCheckoutId;

use super::CliChildFactory;
use super::tests::{Sandbox, checkout, database_view, profile_identity};
use super::tests_database::{runtime_with, selected, sqlite_profile, store};
use crate::commands::coding::database::CodingDatabaseTools;

/// The job's data surface over one selected SQLite profile, with the view that
/// admits it: the ask rides the database, so this is the shape that can offer
/// it at all.
fn ask_surface(
    checkout_id: &ManagedCheckoutId,
    tag: &str,
    clarification: bool,
) -> (CodingDatabaseTools, ChildAuthorityView) {
    let root = std::env::temp_dir().join(format!("saya-coding-ask-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let (profile, _path) = sqlite_profile(&root, "main");
    let runtime = runtime_with(&[("main".to_owned(), profile.clone())]);
    let identity = profile_identity("main", &profile, &runtime.cache_scope);
    let view = database_view(checkout_id, vec![identity.clone()]);
    let tools = CodingDatabaseTools::new(
        runtime,
        store(),
        selected(&[("main".to_owned(), identity)]),
        clarification,
    );
    (tools, view)
}

/// The job's data surface for a job that selected no profile. `clarification`
/// is whether an operator surface is attached to answer an ask.
fn ask_only(clarification: bool) -> CodingDatabaseTools {
    CodingDatabaseTools::new(runtime_with(&[]), store(), BTreeMap::new(), clarification)
}

fn factory_with(
    sandbox: &Sandbox,
    checkout_id: &saya_types::ManagedCheckoutId,
    databases: CodingDatabaseTools,
) -> CliChildFactory {
    CliChildFactory::new(
        sandbox.pair(checkout_id),
        super::tests_factory::test_ai(),
        Arc::new(saya_config::MapSecretResolver::new([])),
    )
    .with_databases(databases)
}

fn advertises(definitions: &[saya_agent::ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|definition| definition.name == name)
}

/// The scripted provider: one scripted turn per request, and the requests it
/// saw. A call past the script pops from an empty vec and panics, so "the turn
/// ended" is proven by the run returning at all.
struct ScriptedProvider {
    turns: Mutex<Vec<ChatMessage>>,
    seen: Mutex<Vec<ChatRequest>>,
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
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.seen.lock().unwrap().push(request);
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

const QUESTION: &str = "Which grain should the daily rollup use?";

fn ask_turn() -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: QUESTION.into(),
        tool_calls: vec![ToolCall {
            id: "call-ask".into(),
            name: saya_agent::REQUEST_CLARIFICATION_TOOL.into(),
            arguments: serde_json::json!({"question": QUESTION, "options": ["per day", "per week"]}),
        }],
        tool_call_id: None,
        images: Vec::new(),
        reasoning: None,
        reasoning_signature: None,
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "roll up the events".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// The operator's answer, as the caller feeds it back: the next turn's user
/// message, over the conversation the ask left behind.
const ANSWER: &str = "Per day.";

fn answer_turn() -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: "Rolling up per day.".into(),
        tool_calls: Vec::new(),
        tool_call_id: None,
        images: Vec::new(),
        reasoning: None,
        reasoning_signature: None,
    }
}

/// The answered turn: the task and the question as history, the answer as the
/// user message the model is about to receive.
fn answered_request() -> AgentRequest {
    AgentRequest {
        prompt: ANSWER.into(),
        history: vec![
            ChatMessage::text("user", "roll up the events"),
            ChatMessage::text("assistant", QUESTION),
        ],
        ..request()
    }
}

/// A helper with no selected profile is offered no ask, whoever is attached:
/// the ask is a database tool, and a job whose work is code alone has none.
#[tokio::test]
async fn a_helper_without_a_profile_is_offered_no_ask() {
    let checkout_id = checkout("checkout");
    let view = database_view(&checkout_id, Vec::new());
    let sandbox = Sandbox::new("ask-advertise-none");
    for clarification in [true, false] {
        let definitions = factory_with(&sandbox, &checkout_id, ask_only(clarification))
            .build_toolset(&view)
            .unwrap()
            .0;
        assert!(
            !advertises(&definitions, "request_clarification"),
            "no profile means no database and no ask (operator: {clarification})"
        );
    }
}

/// A helper with a selected profile advertises the ask when an operator can
/// answer, and nothing at all when none can.
#[tokio::test]
async fn a_helper_with_a_profile_advertises_the_ask_only_for_an_operator() {
    let checkout_id = checkout("checkout");
    let sandbox = Sandbox::new("ask-advertise");
    let (answered_tools, answered_view) = ask_surface(&checkout_id, "advertise", true);
    let answered = factory_with(&sandbox, &checkout_id, answered_tools)
        .build_toolset(&answered_view)
        .unwrap()
        .0;
    assert!(advertises(&answered, "request_clarification"));

    let (headless_tools, headless_view) = ask_surface(&checkout_id, "advertise-headless", false);
    let headless = factory_with(&sandbox, &checkout_id, headless_tools)
        .build_toolset(&headless_view)
        .unwrap()
        .0;
    assert!(
        !advertises(&headless, "request_clarification"),
        "a headless job has nobody to answer, so it must never ask"
    );
}

/// A call ends the turn: the loop stops after one provider request, the
/// question rides `ClarificationNeeded`, and the tool result the model reads
/// says the user's next message is the answer. No connection is ever resolved.
#[tokio::test]
async fn the_ask_ends_the_turn_and_the_next_message_is_the_answer() {
    let checkout_id = checkout("checkout");
    let sandbox = Sandbox::new("ask-turn");
    let (databases, view) = ask_surface(&checkout_id, "turn", true);
    let (definitions, tools, approval) = factory_with(&sandbox, &checkout_id, databases)
        .build_toolset(&view)
        .unwrap();
    assert!(advertises(&definitions, "request_clarification"));

    let provider = ScriptedProvider {
        turns: Mutex::new(vec![ask_turn()]),
        seen: Mutex::new(Vec::new()),
    };
    let output = run_agent_with_sink(
        &provider,
        &tools,
        request(),
        definitions,
        AgentLimits::default(),
        &approval,
        &NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("the turn ends on the ask, never in an error");

    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { question, .. } if question == QUESTION)),
        "the question is on the run: {:?}",
        output.events
    );
    let result = tools
        .execute(
            saya_agent::REQUEST_CLARIFICATION_TOOL,
            serde_json::json!({"question": QUESTION}),
        )
        .await
        .expect("an ask needs no database connection");
    assert_eq!(result["asked"], serde_json::json!(true), "{result}");
    assert!(
        result["next"]
            .as_str()
            .is_some_and(|next| next.contains("next message")),
        "the model is told the user's next message is the answer: {result}"
    );
}

/// The answer side of the same contract at this surface: the ask ends the turn
/// rather than the conversation, so the operator's answer is an ordinary user
/// message — the caller feeds it as the next turn's history and the model
/// receives it. The turn-ending rules themselves are the loop's own
/// clarification tests'; this proves the coding surface's ask leaves the
/// conversation usable.
#[tokio::test]
async fn the_answer_is_the_next_user_message_the_model_receives() {
    let checkout_id = checkout("checkout");
    let sandbox = Sandbox::new("ask-answer");
    let (databases, view) = ask_surface(&checkout_id, "answer", true);
    let (definitions, tools, approval) = factory_with(&sandbox, &checkout_id, databases)
        .build_toolset(&view)
        .unwrap();
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![ask_turn(), answer_turn()]),
        seen: Mutex::new(Vec::new()),
    };
    let asked = run_agent_with_sink(
        &provider,
        &tools,
        request(),
        definitions.clone(),
        AgentLimits::default(),
        &approval,
        &NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("the ask ends the turn");
    assert!(
        asked
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "the run stopped on the question, not on an answer"
    );
    let answered = run_agent_with_sink(
        &provider,
        &tools,
        answered_request(),
        definitions,
        AgentLimits::default(),
        &approval,
        &NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("the answered turn runs");
    assert_eq!(answered.answer, "Rolling up per day.");
    let seen = provider.seen.lock().unwrap();
    let last = seen
        .last()
        .and_then(|request| request.messages.last())
        .expect("the second turn reached the model");
    assert_eq!(last.role, "user");
    assert_eq!(last.content, ANSWER);
}
