//! Read-only calls in one turn overlap; a call with effects keeps the turn
//! serial. `shell_output` is the read-shaped member of the shell family: three
//! reads of three background runs are independent, so the runner may overlap
//! them, while an `edit` in the same turn keeps today's serial order.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, ApprovalDecider, CancellationToken, ChatProvider, ChatRequest,
    ChatResponse, NoopEventSink, ProviderError, ProviderEvent, ProviderStream, ToolCall,
    ToolDefinition, ToolError, ToolExecutor, run_agent_with_sink,
};
use serde_json::json;
use tokio::sync::{Notify, Semaphore};

use crate::agent::tools::DatabaseTools;

use super::schema::{BackgroundOwner, background_definitions};

/// Every read blocks until the third has started, so a serial runner leaves
/// the first waiting on the gate and the test's own timeout fails it instead
/// of hanging.
struct LatchReads {
    started: AtomicUsize,
    all_started: Notify,
    release: Semaphore,
}

impl LatchReads {
    fn new() -> Self {
        Self {
            started: AtomicUsize::new(0),
            all_started: Notify::new(),
            release: Semaphore::new(0),
        }
    }
}

#[async_trait]
impl ToolExecutor for LatchReads {
    async fn execute(
        &self,
        _: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if self.started.fetch_add(1, Ordering::AcqRel) + 1 == 3 {
            self.all_started.notify_one();
        }
        self.release
            .acquire()
            .await
            .expect("the test gate stays open")
            .forget();
        Ok(json!({"id": arguments.get("id").cloned(), "status": "running"}))
    }
}

/// Records how many calls run at the same time.
struct ProbeSerial {
    in_flight: AtomicUsize,
    high_water: AtomicUsize,
}

impl ProbeSerial {
    fn new() -> Self {
        Self {
            in_flight: AtomicUsize::new(0),
            high_water: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ToolExecutor for ProbeSerial {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.high_water.fetch_max(current, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(30)).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(json!({"ok": true}))
    }
}

/// Approves every call. None of these definitions requires a prompt, so this
/// stands in for the decider and nothing else.
struct ApproveAll;

#[async_trait]
impl ApprovalDecider for ApproveAll {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

struct CallsThenAnswer {
    calls: Vec<ToolCall>,
    turn: Mutex<bool>,
}

#[async_trait]
impl ChatProvider for CallsThenAnswer {
    fn name(&self) -> &str {
        "shell-read-concurrency"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the streaming provider path is used")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let first = {
            let mut turn = self.turn.lock().expect("turn counter is available");
            let first = !*turn;
            *turn = true;
            first
        };
        let events = if first {
            vec![
                Ok(ProviderEvent::ToolCalls(self.calls.clone())),
                Ok(ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("done".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "read the background runs".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn read_call(id: u64) -> ToolCall {
    ToolCall {
        id: format!("call-{id}"),
        name: "shell_output".into(),
        arguments: json!({"id": id}),
    }
}

fn shell_output_definition() -> Vec<ToolDefinition> {
    background_definitions(BackgroundOwner::Job)
        .into_iter()
        .filter(|definition| definition.name == "shell_output")
        .collect()
}

#[tokio::test]
async fn three_background_reads_in_one_turn_run_concurrently_and_return_in_order() {
    let tools = Arc::new(LatchReads::new());
    let provider = CallsThenAnswer {
        calls: vec![read_call(1), read_call(2), read_call(3)],
        turn: Mutex::new(false),
    };
    let run = {
        let tools = Arc::clone(&tools);
        tokio::spawn(async move {
            run_agent_with_sink(
                &provider,
                tools.as_ref(),
                request(),
                shell_output_definition(),
                AgentLimits::default(),
                &ApproveAll,
                &NoopEventSink,
                CancellationToken::new(),
            )
            .await
        })
    };
    let overlapped = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if tools.started.load(Ordering::Acquire) == 3 {
                break;
            }
            tools.all_started.notified().await;
        }
    })
    .await
    .is_ok();

    // Release on success and feature-red timeout, so neither future is stranded.
    tools.release.add_permits(3);
    let output = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("the released tool loop must finish")
        .expect("the tool loop task must join")
        .expect("three admitted reads and the final answer must succeed");

    assert!(
        overlapped,
        "three read-only calls must be in flight before either returns"
    );
    let ids: Vec<u64> = output
        .tool_metadata
        .iter()
        .map(|metadata| {
            serde_json::from_str::<serde_json::Value>(&metadata.arguments)
                .expect("the recorded arguments are JSON")["id"]
                .as_u64()
                .expect("the read carries an id")
        })
        .collect();
    assert_eq!(ids, [1, 2, 3], "results are recorded in call order");
}

#[tokio::test]
async fn a_write_in_the_turn_forces_serial_order() {
    let tools = Arc::new(ProbeSerial::new());
    let edit = DatabaseTools::definitions(false, false, false, true)
        .into_iter()
        .find(|definition| definition.name == "edit")
        .expect("edit is advertised when workspace writes are permitted");
    let mut definitions = shell_output_definition();
    definitions.push(edit);
    let provider = CallsThenAnswer {
        calls: vec![
            read_call(1),
            ToolCall {
                id: "call-edit".into(),
                name: "edit".into(),
                arguments: json!({"path": "notes.md", "old_text": "a", "new_text": "b"}),
            },
        ],
        turn: Mutex::new(false),
    };
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        run_agent_with_sink(
            &provider,
            tools.as_ref(),
            request(),
            definitions,
            AgentLimits {
                permit_workspace_writes: true,
                ..AgentLimits::default()
            },
            &ApproveAll,
            &NoopEventSink,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("a mixed turn must finish")
    .expect("a mixed turn must succeed");

    assert_eq!(
        tools.high_water.load(Ordering::SeqCst),
        1,
        "a call with effects keeps the whole turn serial"
    );
    let names: Vec<&str> = output
        .tool_metadata
        .iter()
        .map(|metadata| metadata.name.as_str())
        .collect();
    assert_eq!(names, ["shell_output", "edit"], "call order is preserved");
}
