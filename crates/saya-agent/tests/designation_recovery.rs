//! A `designate_answer` call that arrives with no prose must not end the run
//! with an empty answer: the loop takes one bounded follow-up turn to recover
//! the prose, the designated SQL survives every way the run can end, and the
//! recovery is invisible when the designation already carries its sentence.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentLimits, AgentRequest, AllowReadOnlyApproval, CancellationToken, ChatProvider,
    ChatRequest, LocalStateEffect, ProviderError, ProviderEvent, ProviderStream, ToolCall,
    ToolDefinition, ToolEffect, ToolError, ToolExecutor, run_agent,
};
use std::sync::{Arc, Mutex};

/// A provider that replays a scripted stream per call, records every request,
/// and counts how many times it was called.
struct Scripted {
    turns: Vec<Vec<Result<ProviderEvent, ProviderError>>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl Scripted {
    fn turn(&self, index: usize) -> Vec<Result<ProviderEvent, ProviderError>> {
        self.turns
            .get(index)
            .unwrap_or_else(|| panic!("scripted turn {index} was not provided"))
            .clone()
    }
}

#[async_trait]
impl ChatProvider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }
    async fn complete(&self, _: ChatRequest) -> Result<saya_agent::ChatResponse, ProviderError> {
        unreachable!("designation recovery drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let index = self.requests.lock().unwrap().len() - 1;
        Ok(Box::pin(futures_util::stream::iter(self.turn(index))))
    }
}

struct ExecutedTools {
    names: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ToolExecutor for ExecutedTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.names.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"columns": ["a"], "rows": [[1]]}))
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "show data".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "bounded_sql_query".into(),
        description: "read-only query".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        completion: None,
    }]
}

fn designation_call(id: &str, sql: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "designate_answer".into(),
        arguments: serde_json::json!({"sql": sql}),
    }
}

fn text_then_done(text: &str) -> Vec<Result<ProviderEvent, ProviderError>> {
    vec![
        Ok(ProviderEvent::TextDelta(text.into())),
        Ok(ProviderEvent::Done),
    ]
}

fn calls_only(calls: Vec<ToolCall>) -> Vec<Result<ProviderEvent, ProviderError>> {
    vec![Ok(ProviderEvent::ToolCalls(calls)), Ok(ProviderEvent::Done)]
}

fn designated_events(output: &saya_agent::AgentOutput) -> Vec<&str> {
    output
        .events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::AnswerDesignated { sql } => Some(sql.as_str()),
            _ => None,
        })
        .collect()
}

fn answered_call_ids(request: &ChatRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .filter_map(|message| message.tool_call_id.clone())
        .collect()
}

/// The measured failure: the model sends `designate_answer` with no prose. The
/// run must not end there — the designated SQL is recorded and answered, and
/// one follow-up turn produces the sentence around it.
#[tokio::test]
async fn empty_designation_takes_one_follow_up_turn_for_the_prose() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")]),
            text_then_done("The answer is 42."),
        ],
        requests: requests.clone(),
    };
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("an empty designation recovers its prose in one follow-up turn");
    assert_eq!(
        output.answer, "The answer is 42.",
        "an empty-prose designation must recover the prose in one bounded follow-up turn"
    );
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the designated SQL is carried on the output"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "exactly one follow-up call: the designation turn plus the recovery turn"
    );
    assert_eq!(
        designated_events(&output),
        vec!["SELECT count(*) FROM t"],
        "exactly one AnswerDesignated event, for the first designation"
    );
    let second = &requests.lock().unwrap()[1];
    assert!(
        second.messages.iter().any(|message| {
            message.role == "tool" && message.tool_call_id.as_deref() == Some("d1")
        }),
        "the follow-up request must answer the designation call: {second:?}"
    );
}

/// A designation that arrives alongside its prose is unchanged: the run ends
/// there and the provider is not called again. (Pins existing behaviour.)
#[tokio::test]
async fn designation_with_prose_makes_no_extra_call() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")])
                .into_iter()
                .chain(std::iter::once(Ok(ProviderEvent::TextDelta(
                    "The answer is 42.".into(),
                ))))
                .collect(),
        ],
        requests: requests.clone(),
    };
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a designation with prose completes the run");
    assert_eq!(output.answer, "The answer is 42.");
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "a designation with prose must end the run without an extra provider call"
    );
}

/// The bound is one recovery per run: a follow-up turn that also designates
/// with no prose completes with what the run already has — the first
/// designation — instead of looping.
#[tokio::test]
async fn a_second_empty_designation_does_not_recover_again() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")]),
            calls_only(vec![designation_call("d2", "SELECT count(*) FROM other")]),
        ],
        requests: requests.clone(),
    };
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a second empty designation completes rather than recovering again");
    assert_eq!(
        output.answer, "",
        "no prose was ever produced; the run must not invent one"
    );
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the first designation wins"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "one recovery turn only: the designation turn plus the follow-up"
    );
    assert_eq!(
        designated_events(&output).len(),
        1,
        "the AnswerDesignated event is emitted exactly once per run"
    );
}

/// The follow-up turn failing must not fail the run: a reply with neither
/// content nor tool calls is fatal for an ordinary turn, but after an empty
/// designation the run still ends with the SQL it already has.
#[tokio::test]
async fn an_empty_follow_up_reply_keeps_the_designation() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")]),
            vec![Ok(ProviderEvent::Done)],
        ],
        requests: requests.clone(),
    };
    let output = match run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    {
        Ok(output) => output,
        Err(error) => panic!("a failed follow-up turn must not fail the run: {error}"),
    };
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "the follow-up turn must actually run and fail, not be skipped"
    );
    assert_eq!(output.answer, "");
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the designation survives the failed follow-up"
    );
}

/// Every tool call riding the same message as an empty designation is answered
/// — a provider rejects a request whose assistant tool calls lack replies —
/// and none of them is executed: the run already has its answer.
#[tokio::test]
async fn sibling_calls_of_an_empty_designation_are_all_answered() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![
                ToolCall {
                    id: "b1".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({"sql": "SELECT 1"}),
                },
                designation_call("d1", "SELECT count(*) FROM t"),
            ]),
            text_then_done("The answer is 42."),
        ],
        requests: requests.clone(),
    };
    let executed = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: executed.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the designation recovers its prose");
    assert_eq!(
        output.answer, "The answer is 42.",
        "an empty-prose designation must recover the prose"
    );
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "the designation turn plus one follow-up turn"
    );
    let answered = answered_call_ids(&requests[1]);
    for id in ["b1", "d1"] {
        assert!(
            answered.iter().any(|seen| seen == id),
            "the follow-up request must answer call {id}: {answered:?}"
        );
    }
    assert!(
        executed.lock().unwrap().is_empty(),
        "the sibling call must not be executed: the answer was already designated"
    );
}

/// A follow-up that re-designates different SQL keeps the first designation —
/// the event and the output cannot disagree about which query answers the
/// question. (Pins the first-designation-wins rule.)
#[tokio::test]
async fn a_re_designation_keeps_the_first_sql() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")]),
            calls_only(vec![designation_call("d2", "SELECT count(*) FROM other")])
                .into_iter()
                .chain(std::iter::once(Ok(ProviderEvent::TextDelta(
                    "The answer is 42.".into(),
                ))))
                .collect(),
        ],
        requests: requests.clone(),
    };
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the re-designation carries prose and ends the run");
    assert_eq!(output.answer, "The answer is 42.");
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the first designation wins over the re-designation"
    );
    assert_eq!(
        designated_events(&output),
        vec!["SELECT count(*) FROM t"],
        "exactly one AnswerDesignated event, for the first designation"
    );
}

/// A designation recovered into a turn that hits the turn ceiling still
/// carries its SQL: the salvage that ends the run is built on the work the
/// designation already committed.
#[tokio::test]
async fn recovery_at_the_turn_ceiling_still_carries_the_designation() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")]),
            text_then_done("salvaged prose"),
        ],
        requests: requests.clone(),
    };
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(1),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the ceiling salvages the run instead of failing it");
    assert!(
        output.truncated,
        "the run was salvaged at the turn ceiling, not completed"
    );
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the salvage carries the designated SQL"
    );
}
