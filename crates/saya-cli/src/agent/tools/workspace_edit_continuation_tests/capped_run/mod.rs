use super::*;

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

mod capped_write;
mod stale_resume;
mod stream_cap;
