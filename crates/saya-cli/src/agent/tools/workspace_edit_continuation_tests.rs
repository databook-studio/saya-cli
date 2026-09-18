//! End-to-end: a run capped mid-file-write resumes with `workspace_edit`'s
//! append variant and the finished file is byte-exact.
//!
//! Why this file lives here, not in `crates/saya-cli/tests/`: the claim needs
//! a real [`Workspace`] wired to the real tool registry, and the `Sandbox`
//! helper in `workspace_edit_tests`/`workspace_edit_append_tests` is exactly
//! that wiring — a temp-dir workspace plus `DatabaseTools::with_registry`.
//! Those unit-test helpers are private to their modules, so this file keeps
//! its own small copy rather than reaching across test modules. The loop
//! itself (`run_agent_with_sink`) is crate-public, so no production change is
//! needed to drive it from here.

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use saya_agent::{
    AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, CancellationToken,
    ChatMessage, ChatProvider, ChatRequest, ToolCall, ToolExecutor,
};

use super::database_tools::DatabaseTools;
use saya_harness::workspace::Workspace;

/// The two chunks the model writes across the cap. Plain ASCII, no
/// credential-shaped substrings, so transcript redaction cannot move bytes
/// under the assertions.
const CHUNK_ONE: &str = "line one: the capped write;\n";
const CHUNK_TWO: &str = "line two: the resumed write;\n";
/// Half of chunk one the provider claims it already emitted before the cap.
/// Distinct from both chunks so a replay would be visible.
const PARTIAL_TEXT: &str = "PARTIAL-CAPPED-PROSE-SHOULD-BE-DISCARDED-9k2v";
/// The unfinished tool-argument fragment the cap interrupts. A truncated JSON
/// object on purpose: it must never parse, never execute, never replay.
const PARTIAL_TOOL_JSON: &str =
    "{\"path\": \"report.txt\", \"offset\": 0, \"chunk\": \"line one: the cap";
/// The cooperative answer ending the run. Distinct from every fragment above
/// so the final-answer assertion cannot pass on salvaged prose.
const FINAL_ANSWER: &str = "the report is complete";
/// Writer used by the stream-cap test: must trip `MAX_STREAM_BYTES` on one
/// delta so the accumulation guard fires inside one attempt.
fn over_stream_cap_text() -> String {
    "S".repeat(saya_agent::MAX_STREAM_BYTES + 1)
}

/// A sandbox workspace under the OS temp dir, removed on drop. Mirrors the
/// helper in the neighbouring workspace-edit test modules.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wscont-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn ws_root(&self) -> PathBuf {
        self.outer.join("ws")
    }

    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

/// The real definition, taken from the advertised list: the loop must run
/// against the definition the model would actually see.
fn workspace_edit_definition() -> saya_agent::ToolDefinition {
    DatabaseTools::definitions(false, false, false, true, false)
        .into_iter()
        .find(|tool| tool.name == "workspace_edit")
        .expect("workspace_edit is advertised when workspace writes are permitted")
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "write the report".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn limits() -> AgentLimits {
    AgentLimits {
        permit_workspace_writes: true,
        ..AgentLimits::default()
    }
}

/// A tool executor that delegates to the real `DatabaseTools` while recording
/// every `workspace_edit` call's arguments, so the test can inspect what the
/// resume turn actually sent.
struct SpiedTools {
    inner: DatabaseTools,
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
}

#[async_trait::async_trait]
impl ToolExecutor for SpiedTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, saya_agent::ToolError> {
        if name == "workspace_edit" {
            self.calls.lock().unwrap().push(arguments.clone());
        }
        self.inner.execute(name, arguments).await
    }
}

/// A scripted provider with four turns: turn 1 writes the first chunk (a
/// completed tool turn, so the harness's `size`/`digest` result is in the
/// conversation); turn 2 streams a text delta and then caps with
/// `OutputTruncated` — the partial is discarded and nothing executes; turn 3
/// emits the append call whose `offset`/`expected_digest` are the
/// size/digest turn 1's tool result reported; turn 4 answers in prose, ending
/// the run. The turn-3 call is built lazily from the tool result turn 1
/// produced — the provider reads its own request history for the
/// `size`/`digest` the harness reported, exactly as the model would.
struct CappedWriteProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    calls: Mutex<usize>,
}

fn append_call(offset: u64, digest: &str) -> ToolCall {
    ToolCall {
        id: "resume-1".into(),
        name: "workspace_edit".into(),
        arguments: serde_json::json!({
            "path": "report.txt",
            "offset": offset,
            "expected_digest": digest,
            "chunk": CHUNK_TWO,
        }),
    }
}

/// Finds the turn-1 `workspace_edit` result inside a request's messages: the
/// tool-role message answering the first assistant turn's call.
fn turn_one_result(messages: &[ChatMessage]) -> serde_json::Value {
    let tool_message = messages
        .iter()
        .find(|message| message.role == "tool")
        .expect("turn 1's tool result must be in the turn-2 request");
    serde_json::from_str(&tool_message.content).expect("the tool result is JSON")
}

#[async_trait::async_trait]
impl ChatProvider for CappedWriteProvider {
    fn name(&self) -> &str {
        "capped-write-script"
    }
    async fn complete(
        &self,
        _: ChatRequest,
    ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<saya_agent::ProviderStream, saya_agent::ProviderError> {
        let turn = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls
        };
        self.requests.lock().unwrap().push(request.clone());
        match turn {
            1 => Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(vec![ToolCall {
                    id: "write-1".into(),
                    name: "workspace_edit".into(),
                    arguments: serde_json::json!({
                        "path": "report.txt",
                        "offset": 0,
                        "chunk": CHUNK_ONE,
                    }),
                }])),
                Ok(saya_agent::ProviderEvent::Done),
            ]))),
            // The cap lands on a prose turn after the write already committed:
            // the partial is discarded, nothing executes, and the file — not
            // the conversation — is the source of truth for the resume.
            2 => Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(saya_agent::ProviderEvent::TextDelta(
                    "finishing the sentence ".into(),
                )),
                Err(saya_agent::ProviderError::output_truncated(
                    PARTIAL_TEXT.into(),
                    vec![PARTIAL_TOOL_JSON.into()],
                )),
            ]))),
            3 => {
                let result = turn_one_result(&request.messages);
                let offset = result["size"]
                    .as_u64()
                    .expect("the harness reports the size turn 2 resumes from");
                let digest = result["digest"]
                    .as_str()
                    .expect("the harness reports the digest turn 2 resumes from")
                    .to_owned();
                Ok(Box::pin(futures_util::stream::iter(vec![
                    Ok(saya_agent::ProviderEvent::ToolCalls(vec![append_call(
                        offset, &digest,
                    )])),
                    Ok(saya_agent::ProviderEvent::Done),
                ])))
            }
            _ => Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(saya_agent::ProviderEvent::TextDelta(FINAL_ANSWER.into())),
                Ok(saya_agent::ProviderEvent::Done),
            ]))),
        }
    }
}

struct NoopSink;

#[async_trait::async_trait]
impl AgentEventSink for NoopSink {
    async fn emit(&self, _: saya_agent::AgentEvent) {}
}

/// Counts user-role messages carrying the continuation note, keyed on its
/// fixed wording rather than on the loop's private constant.
fn continuation_notes(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .filter(|message| message.role == "user" && message.content.contains("output-token limit"))
        .count()
}

/// The claim: a run capped mid-file-write resumes with the append variant
/// using the `size`/`digest` the harness reported, and the finished file is
/// byte-exact — chunk one followed by chunk two, no duplication, no gap.
#[tokio::test]
async fn a_capped_run_finishes_the_file_by_appending() {
    let sandbox = Sandbox::new("capped-append");
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let tools = SpiedTools {
        inner: sandbox.tools(),
        calls: tool_calls.clone(),
    };
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CappedWriteProvider {
        requests: requests.clone(),
        calls: Mutex::new(0),
    };
    let output = saya_agent::run_agent_with_sink(
        &provider,
        &tools,
        request(),
        vec![workspace_edit_definition()],
        limits(),
        &AllowReadOnlyApproval,
        &NoopSink,
        CancellationToken::new(),
    )
    .await
    .expect("the capped run must finish after the resume");
    assert_eq!(
        output.answer, FINAL_ANSWER,
        "the run's final answer is the last turn's, not a salvaged one"
    );
    let expected = format!("{CHUNK_ONE}{CHUNK_TWO}");
    let actual = fs::read(sandbox.ws_root().join("report.txt")).expect("the file must exist");
    assert_eq!(
        actual,
        expected.as_bytes(),
        "the finished file is byte-exact: chunk one followed by chunk two"
    );
    let seen_calls = tool_calls.lock().unwrap().clone();
    assert_eq!(
        seen_calls.len(),
        2,
        "exactly two workspace_edit calls: the first chunk and the resume"
    );
    assert_eq!(
        seen_calls[1]["chunk"], CHUNK_TWO,
        "the resume sends only the second chunk, never resending turn 1's bytes"
    );
    assert!(
        !seen_calls[1].to_string().contains(CHUNK_ONE),
        "turn 1's bytes appear nowhere in the resume call: {}",
        seen_calls[1]
    );
    assert_eq!(
        seen_calls[1]["offset"],
        CHUNK_ONE.len() as u64,
        "the resume offset is the size turn 1's tool result reported"
    );
    let seen_requests = requests.lock().unwrap().clone();
    assert_eq!(
        seen_requests.len(),
        4,
        "first write, cap, resume, and final answer"
    );
    // The continuation note follows the cap: it is in the resume turn's
    // request (index 2), and the partial must be absent there.
    let turn_two = seen_requests[2].messages.clone();
    for message in &turn_two {
        assert!(
            !message.content.contains(PARTIAL_TEXT),
            "the capped turn's partial text appears nowhere the provider saw on the resume: {}",
            message.content
        );
        let calls = serde_json::to_string(&message.tool_calls).unwrap();
        assert!(
            !calls.contains(PARTIAL_TEXT) && !calls.contains(PARTIAL_TOOL_JSON),
            "no partial may hide in a replayed tool call: {calls}"
        );
    }
    assert!(
        !turn_two
            .iter()
            .any(|message| message.role == "tool" && message.content.contains(PARTIAL_TEXT)),
        "the partial is not smuggled in through a tool message either"
    );
    assert_eq!(
        continuation_notes(&turn_two),
        1,
        "the continuation note appears exactly once"
    );
    // The resume's precondition is the digest turn 1's tool result reported:
    // re-read that result from the resume request's own messages (what the
    // model saw) and compare against what the resume call actually sent.
    let reported = turn_one_result(&turn_two);
    assert!(
        reported["digest"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "the tool result the model sees carries the digest: {reported}"
    );
    assert_eq!(
        seen_calls[1]["expected_digest"], reported["digest"],
        "the resume echoes the digest the harness reported, not a guess"
    );
}

/// The resume path inherits the stale-offset refusal: a continuation that
/// appends at a stale offset refuses, reports the current size and digest,
/// and leaves the file byte-identical. Precedent:
/// `workspace_edit_append_tests.rs:59-92`.
#[tokio::test]
async fn a_stale_resume_offset_refuses_and_preserves_bytes() {
    let sandbox = Sandbox::new("stale-resume");
    sandbox
        .ws
        .write("report.txt", CHUNK_ONE.as_bytes())
        .expect("seed write must succeed");
    let before = fs::read(sandbox.ws_root().join("report.txt")).expect("seed file must exist");
    let tools = sandbox.tools();
    let stale = before.len().saturating_sub(1) as u64;
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "report.txt",
                "offset": stale,
                "chunk": CHUNK_TWO,
            }),
        )
        .await
        .expect_err("a stale resume offset must refuse, not append");
    let text = error.to_string();
    assert!(
        text.contains(&CHUNK_ONE.len().to_string()),
        "the refusal must report the current size so the model can resume, got: {text}"
    );
    assert!(
        text.contains("digest"),
        "the refusal must carry the current digest for the resume precondition, got: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("report.txt")).expect("file must survive"),
        before,
        "a refused resume leaves the file byte-identical"
    );
}

/// `MAX_STREAM_BYTES` still bounds a stream on the continuation path: a text
/// delta over the cap fails the turn with the size-limit error before any
/// continuation note is pushed, so the resume path cannot smuggle an
/// unbounded stream past the cap.
#[tokio::test]
async fn the_continuation_path_does_not_bypass_the_stream_cap() {
    struct CappedProvider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }
    #[async_trait::async_trait]
    impl ChatProvider for CappedProvider {
        fn name(&self) -> &str {
            "oversized-stream"
        }
        async fn complete(
            &self,
            _: ChatRequest,
        ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
            unreachable!("the main loop drives the provider through stream")
        }
        async fn stream(
            &self,
            request: ChatRequest,
            _: CancellationToken,
        ) -> Result<saya_agent::ProviderStream, saya_agent::ProviderError> {
            self.requests.lock().unwrap().push(request);
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(saya_agent::ProviderEvent::TextDelta(over_stream_cap_text())),
                Ok(saya_agent::ProviderEvent::Done),
            ])))
        }
    }
    let sandbox = Sandbox::new("stream-cap");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CappedProvider {
        requests: requests.clone(),
    };
    let error = saya_agent::run_agent_with_sink(
        &provider,
        &sandbox.tools(),
        request(),
        vec![workspace_edit_definition()],
        limits(),
        &AllowReadOnlyApproval,
        &NoopSink,
        CancellationToken::new(),
    )
    .await
    .expect_err("an oversized stream must fail the run");
    assert!(
        error.to_string().contains("size limit"),
        "the stream cap must fire, got: {error}"
    );
    assert!(
        !sandbox.ws_root().join("report.txt").exists(),
        "the oversized turn ran no tool: no file may exist"
    );
    for seen in requests.lock().unwrap().iter() {
        assert_eq!(
            continuation_notes(&seen.messages),
            0,
            "no continuation note may be pushed for a stream-cap failure"
        );
    }
}
