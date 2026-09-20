//! C0 — name the call: each completion line carries the call's key fact.
//!
//! The summary half of the properties, placed beside the summaries it pins
//! (integration over the loop's public surface, per the testing standard):
//! a `workspace_write` completion names the file, a `run_command`
//! completion names the program and its outcome, a failed call still reads
//! as failed, and an uncovered tool keeps today's text byte-exact. The
//! request half (shared detail seam, both adapters) lives beside the seam
//! in `saya-cli/src/agent/tools/tool_calls.rs`.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, ChatProvider, ChatRequest, ChatResponse,
    LocalStateEffect, ToolCall, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
    run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

struct OneCallProvider {
    call: ToolCall,
    turn: Mutex<u32>,
}

#[async_trait]
impl ChatProvider for OneCallProvider {
    fn name(&self) -> &str {
        "one-call-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: saya_agent::CancellationToken,
    ) -> Result<saya_agent::ProviderStream, saya_agent::ProviderError> {
        let first = {
            let mut turn = self.turn.lock().unwrap();
            let was = *turn;
            *turn += 1;
            was == 0
        };
        let events = if first {
            vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(vec![
                    self.call.clone(),
                ])),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(saya_agent::ProviderEvent::TextDelta("done".into())),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

struct OkExecutor {
    result: serde_json::Value,
}

#[async_trait]
impl ToolExecutor for OkExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(self.result.clone())
    }
}

struct FailExecutor;

#[async_trait]
impl ToolExecutor for FailExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Err(ToolError::QueryFailed)
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

struct ApproveAll;

#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for ApproveAll {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

fn workspace_write_tool() -> ToolDefinition {
    ToolDefinition {
        name: "workspace_write".into(),
        description: "write one file".into(),
        read_only: false,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        },
        completion: Some("workspace file written".into()),
    }
}

fn run_command_tool() -> ToolDefinition {
    ToolDefinition {
        name: "run_command".into(),
        description: "run one program".into(),
        read_only: false,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        },
        completion: Some("host command ran".into()),
    }
}

fn uncovered_tool() -> ToolDefinition {
    ToolDefinition {
        name: "workspace_list".into(),
        description: "list one directory".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::Read,
        },
        completion: Some("workspace directory listed".into()),
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "do something".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

async fn run_one(
    tool: ToolDefinition,
    arguments: serde_json::Value,
    executor: &dyn ToolExecutor,
) -> Vec<AgentEvent> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: tool.name.clone(),
            arguments,
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let limits = AgentLimits {
        permit_candidate_writes: true,
        permit_workspace_writes: true,
        permit_external_effects: true,
        ..AgentLimits::default()
    };
    run_agent_with_sink(
        &provider,
        executor,
        request(),
        vec![tool],
        limits,
        &ApproveAll,
        &sink,
        saya_agent::CancellationToken::new(),
    )
    .await
    .expect("run completes");
    std::mem::take(&mut *events.lock().unwrap())
}

fn completed_summary(events: &[AgentEvent]) -> Option<&str> {
    events.iter().find_map(|event| match event {
        AgentEvent::ToolCompleted { summary, .. } => Some(summary.as_str()),
        _ => None,
    })
}

fn tool_status(output: &saya_agent::AgentOutput) -> &str {
    output.tool_metadata[0].status.as_str()
}

/// Property 1 (completion half): a `workspace_write` completion names the
/// file it wrote.
#[tokio::test]
async fn workspace_write_completion_names_the_file_it_wrote() {
    let arguments = serde_json::json!({"path": "notes.md", "content": "hi"});
    let events = run_one(
        workspace_write_tool(),
        arguments,
        &OkExecutor {
            result: serde_json::json!({"path": "notes.md", "bytes_written": 2}),
        },
    )
    .await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert_eq!(
        summary, "notes.md written",
        "workspace_write completion must name the file: got \"{summary}\""
    );
}

/// Property 2 (completion half): a `run_command` completion names the
/// program and its outcome.
#[tokio::test]
async fn run_command_completion_names_the_program_and_its_outcome() {
    let arguments = serde_json::json!({"program": "pytest", "args": ["-q"]});
    let events = run_one(
        run_command_tool(),
        arguments,
        &OkExecutor {
            result: serde_json::json!({"program": "pytest", "exit_code": 1}),
        },
    )
    .await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert_eq!(
        summary, "pytest exited 1",
        "run_command completion must name the program and outcome: got \"{summary}\""
    );
}

/// Property 3: a failed call is still classified as failed — the substring
/// contract holds. Both the status derivation (`contains("failed")`) and the
/// summary text pin it, for both covered tools.
#[tokio::test]
async fn a_failed_call_is_still_classified_as_failed() {
    for (tool, arguments) in [
        (
            workspace_write_tool(),
            serde_json::json!({"path": "notes.md", "content": "hi"}),
        ),
        (
            run_command_tool(),
            serde_json::json!({"program": "pytest", "args": ["-q"]}),
        ),
    ] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let provider = OneCallProvider {
            call: ToolCall {
                id: "c1".into(),
                name: tool.name.clone(),
                arguments: arguments.clone(),
            },
            turn: Mutex::new(0),
        };
        let sink = RecordingSink {
            events: events.clone(),
        };
        let limits = AgentLimits {
            permit_candidate_writes: true,
            permit_workspace_writes: true,
            permit_external_effects: true,
            ..AgentLimits::default()
        };
        let output = run_agent_with_sink(
            &provider,
            &FailExecutor,
            request(),
            vec![tool.clone()],
            limits,
            &ApproveAll,
            &sink,
            saya_agent::CancellationToken::new(),
        )
        .await
        .expect("run completes");
        let events = events.lock().unwrap();
        let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
        assert!(
            summary.contains("failed"),
            "{} failure summary must keep \"failed\": got \"{summary}\"",
            tool.name
        );
        assert_eq!(
            tool_status(&output),
            "failed",
            "{} failure status must still read failed",
            tool.name
        );
    }
}

/// Property 4 (completion half): a failure line carries no success verb.
/// A failed call must not reuse the success completion text at all — no
/// "written", no "ran", no "listed" — for either covered tool. The bounded,
/// redacted error reason (Phase 7 packet 2) may follow the key fact after
/// " — ", but the success text itself never appears.
#[tokio::test]
async fn a_failure_line_carries_no_success_verb() {
    for (tool, arguments, success_text, expected_prefix) in [
        (
            workspace_write_tool(),
            serde_json::json!({"path": "notes.md", "content": "hi"}),
            "workspace file written",
            "failed notes.md",
        ),
        (
            run_command_tool(),
            serde_json::json!({"program": "pytest", "args": ["-q"]}),
            "host command ran",
            "failed pytest",
        ),
    ] {
        let events = run_one(tool.clone(), arguments, &FailExecutor).await;
        let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
        assert!(
            summary.starts_with(expected_prefix),
            "{} failure summary must name the call without the success text: got \"{summary}\"",
            tool.name
        );
        for verb in ["written", "ran", "listed"] {
            assert!(
                !summary.contains(verb),
                "{} failure summary must not contain the success verb \"{verb}\": got \"{summary}\"",
                tool.name
            );
        }
        assert!(
            !summary.contains(success_text),
            "{} failure summary must not carry the success completion text: got \"{summary}\"",
            tool.name
        );
    }
}

/// Property 5 (completion half): a tool this slice does not cover keeps
/// today's text, byte-exact.
#[tokio::test]
async fn an_uncovered_tool_keeps_todays_text_byte_exact() {
    let events = run_one(
        uncovered_tool(),
        serde_json::json!({}),
        &OkExecutor {
            result: serde_json::json!({"entries": []}),
        },
    )
    .await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert_eq!(
        summary, "workspace directory listed",
        "an uncovered tool must keep today's text byte-exact: got \"{summary}\""
    );
    let events = run_one(uncovered_tool(), serde_json::json!({}), &FailExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.starts_with("failed to complete: workspace directory listed"),
        "an uncovered tool must keep today's failure text as the prefix: got \"{summary}\""
    );
}

/// Extra: the completion line never carries tool output — stdout/stderr and
/// file content stay out of the summary; only the typed key facts name it.
#[tokio::test]
async fn completion_detail_never_carries_tool_output() {
    let events = run_one(
        run_command_tool(),
        serde_json::json!({"program": "pytest"}),
        &OkExecutor {
            result: serde_json::json!({
                "program": "pytest",
                "exit_code": 0,
                "stdout": {"text": "SECRET-STDOUT"},
                "stderr": {"text": "SECRET-STDERR"},
            }),
        },
    )
    .await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert_eq!(summary, "pytest exited 0");
    assert!(
        !summary.contains("SECRET-STDOUT") && !summary.contains("SECRET-STDERR"),
        "tool output must not reach the line: got \"{summary}\""
    );
}
