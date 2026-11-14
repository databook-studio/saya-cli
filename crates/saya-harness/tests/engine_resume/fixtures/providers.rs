/// A per-test scratch root: the state database and the run directory both
/// live under it, and one cleanup covers both.
fn temp_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-engine-resume-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn step(goal: &str) -> StepSpec {
    StepSpec::new(goal, Capabilities::default(), None, Vec::new(), None).unwrap()
}

/// The two-step plan the prefix fixture is driven with.
fn two_step_plan() -> RunPlan {
    RunPlan::new(vec![step("read the schema"), step("write the summary")]).unwrap()
}

/// A provider that answers every request — one request per step driven —
/// and records each one, so tests count exactly the work resume did.
#[derive(Default)]
struct AnswerProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl AnswerProvider {
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl ChatProvider for AnswerProvider {
    fn name(&self) -> &str {
        "answer"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}

/// A provider whose every answer reports usage — the spend a resumed
/// episode's turn makes — ten tokens per call, recorded like its sibling
/// above so tests can count the work.
#[derive(Default)]
struct UsageProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl UsageProvider {
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl ChatProvider for UsageProvider {
    fn name(&self) -> &str {
        "usage-answer"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut response = ChatResponse::new(ChatMessage::text("assistant", "done"));
        response.usage = Some(TokenUsage::new(10, 0));
        Ok(response)
    }
}

/// No scripted turn ever calls a tool; executing one is a test failure.
#[derive(Default)]
struct NoTools {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        panic!("no resume test may execute a tool call")
    }
}

struct HeldResumeTool {
    started: std::sync::Mutex<Option<oneshot::Sender<CancellationToken>>>,
    stopped: std::sync::Mutex<Option<oneshot::Sender<CancellationReason>>>,
    release: std::sync::Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl ToolExecutor for HeldResumeTool {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        unreachable!("the resumed loop must forward its execution context")
    }

    async fn execute_with_context(
        &self,
        _: &str,
        _: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        let _ = self
            .started
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(context.cancellation.clone());
        context.cancellation.cancelled().await;
        let _ = self.stopped.lock().unwrap().take().unwrap().send(
            context
                .cancellation
                .reason()
                .expect("the source sees the driver's stop"),
        );
        let release = self.release.lock().unwrap().take().unwrap();
        release.await.unwrap();
        Ok(serde_json::json!({"settled": true}))
    }
}

/// Auto-approves everything; the stub tools declare no approval need.
#[derive(Default)]
struct AllowApproval;

#[async_trait]
impl ApprovalDecider for AllowApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}
