use async_trait::async_trait;
use saya_agent::*;
use saya_harness::workspace::{ControlledRemoveError, Workspace};
use serde_json::{Value, json};
use std::{
    fs,
    num::NonZeroU64,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub struct Root(pub PathBuf);
impl Root {
    pub fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "saya-delete-effects-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("victim.rs"), b"ordinary source\n").unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub struct Files(pub Workspace);
#[async_trait]
impl ToolExecutor for Files {
    async fn execute(&self, name: &str, args: Value) -> Result<Value, ToolError> {
        self.execute_with_context(
            name,
            args,
            ToolExecutionContext {
                result_cap: 65536,
                cancellation: CancellationToken::new(),
            },
        )
        .await
    }
    async fn execute_with_context(
        &self,
        name: &str,
        args: Value,
        context: ToolExecutionContext,
    ) -> Result<Value, ToolError> {
        assert_eq!(name, "delete");
        assert_eq!(args["path"], "victim.rs");
        let digest = args["expected_digest"].as_str().unwrap();
        let result = self
            .0
            .prepare_guarded_remove(
                "victim.rs",
                digest,
                context.cancellation,
                Instant::now() + Duration::from_secs(5),
            )
            .and_then(|prepared| prepared.commit());
        match result {
            Ok(()) => Ok(json!({"path":"victim.rs","removed":true,"removed_digest":digest})),
            Err(error @ ControlledRemoveError::NoEffect(_)) => Err(ToolError::NoEffect(Box::new(
                ToolError::Runner(error.to_string()),
            ))),
            Err(_) => Err(ToolError::Runner(
                "workspace deletion outcome could not be confirmed".into(),
            )),
        }
    }
}
pub struct Provider {
    pub calls: AtomicUsize,
    pub requests: Mutex<Vec<ChatRequest>>,
    pub digest: String,
}
#[async_trait]
impl ChatProvider for Provider {
    fn name(&self) -> &str {
        "delete-effects-script"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let message = if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_call_id: None,
                tool_calls: vec![ToolCall {
                    id: "actual-delete".into(),
                    name: "delete".into(),
                    arguments: json!({"path":"victim.rs","expected_digest":self.digest}),
                }],

                images: Vec::new(),
                reasoning: None,
                reasoning_signature: None,
            }
        } else {
            ChatMessage::text("assistant", "observed tool outcome")
        };
        Ok(ChatResponse::new(message))
    }
}
pub struct Owner {
    next: AtomicU64,
    pub tools: Mutex<Vec<EffectOutcome>>,
    pub fail: bool,
    pub target: PathBuf,
}
impl Owner {
    pub fn new(target: PathBuf, fail: bool) -> Self {
        Self {
            next: AtomicU64::new(1),
            tools: Mutex::new(Vec::new()),
            fail,
            target,
        }
    }
    fn permit(&self) -> EffectPermit {
        EffectPermit::new(NonZeroU64::new(self.next.fetch_add(1, Ordering::AcqRel)).unwrap())
    }
}
#[async_trait]
impl EffectAdmission for Owner {
    async fn admit_model_request(&self) -> Result<EffectPermit, EffectAdmissionError> {
        Ok(self.permit())
    }
    async fn admit_tool_batch(
        &self,
        effects: &[ToolEffect],
    ) -> Result<Vec<EffectPermit>, EffectAdmissionError> {
        Ok(effects.iter().map(|_| self.permit()).collect())
    }
    async fn settle(
        &self,
        _: EffectPermit,
        settlement: EffectSettlement,
    ) -> Result<(), EffectOwnerFailure> {
        if let EffectSettlement::Tool { outcome } = settlement {
            assert!(
                !self.target.exists(),
                "the real unlink must precede its settlement attempt"
            );
            self.tools.lock().unwrap().push(outcome);
            if self.fail {
                return Err(EffectOwnerFailure::new(std::io::Error::other(
                    "synthetic authoritative settlement failure",
                )));
            }
        }
        Ok(())
    }
}
pub struct Sink {
    pub token: CancellationToken,
    pub target: PathBuf,
    pub cancel: bool,
}
#[async_trait]
impl AgentEventSink for Sink {
    async fn emit(&self, _: AgentEvent) {}
    async fn observe_tool(&self, diagnostic: ToolDiagnostic) {
        if self.cancel
            && diagnostic.tool() == "delete"
            && diagnostic.status() == ToolDiagnosticStatus::Completed
        {
            assert!(
                !self.target.exists(),
                "cancellation is deliberately after the actual unlink"
            );
            self.token.cancel();
        }
    }
}
pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "delete".into(),
        description: "delete the exact fixture file".into(),
        parameters: json!({"type":"object"}),
        read_only: false,
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }]
}
pub fn request() -> AgentRequest {
    AgentRequest {
        prompt: "Delete the one approved fixture file using its observed digest".into(),
        profile_names: vec![],
        model: "fixture".into(),
        system_prompt: None,
        history: vec![],
        context_blocks: vec![],
    }
}
