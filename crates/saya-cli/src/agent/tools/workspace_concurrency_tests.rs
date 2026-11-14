use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatProvider, ChatRequest, ChatResponse,
    NoopEventSink, ProviderError, ProviderEvent, ProviderStream, ToolCall, ToolDefinition,
    ToolError, ToolExecutor, run_agent_with_sink,
};
use saya_harness::workspace::Workspace;
use tokio::sync::{Notify, Semaphore};

use super::DatabaseTools;

struct Sandbox {
    outer: PathBuf,
    workspace: Arc<Workspace>,
}

impl Sandbox {
    fn new() -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-workspace-concurrent-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(&outer).expect("sandbox directory must create");
        let workspace = Arc::new(Workspace::open(&outer).expect("workspace must open"));
        Self { outer, workspace }
    }

    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::clone(&self.workspace)))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

struct ReadGate {
    arrivals: AtomicUsize,
    all_arrived: Notify,
    release: Semaphore,
}

struct GatedReads(DatabaseTools, Arc<ReadGate>);

#[async_trait]
impl ToolExecutor for GatedReads {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let result = self.0.execute(name, arguments).await?;
        if name == "read" {
            if self.1.arrivals.fetch_add(1, Ordering::AcqRel) + 1 == 4 {
                self.1.all_arrived.notify_one();
            }
            self.1
                .release
                .acquire()
                .await
                .expect("test gate remains open")
                .forget();
        }
        Ok(result)
    }
}

struct FourReadsProvider {
    calls: Vec<ToolCall>,
    turn: std::sync::Mutex<u8>,
}

#[async_trait]
impl ChatProvider for FourReadsProvider {
    fn name(&self) -> &str {
        "workspace-read-pair"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the streaming provider path is used")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: saya_agent::CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let first = {
            let mut turn = self.turn.lock().expect("turn counter is available");
            let first = *turn == 0;
            *turn += 1;
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
        prompt: "read both files".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

#[tokio::test]
async fn independent_workspace_reads_reach_the_loop_concurrency_gate() {
    let sandbox = Sandbox::new();
    for (path, bytes) in [
        ("first.txt", b"first".as_slice()),
        ("second.txt", b"second".as_slice()),
        ("third.txt", b"third".as_slice()),
        ("fourth.txt", b"fourth".as_slice()),
    ] {
        sandbox.workspace.write(path, bytes).unwrap();
    }
    let tools = sandbox.tools();
    let definitions: Vec<ToolDefinition> = DatabaseTools::definitions(false, false, false, false)
        .into_iter()
        .filter(|definition| definition.name == "read")
        .collect();
    assert_eq!(
        definitions.len(),
        1,
        "use the actual advertised read definition"
    );
    let gate = Arc::new(ReadGate {
        arrivals: AtomicUsize::new(0),
        all_arrived: Notify::new(),
        release: Semaphore::new(0),
    });
    let provider = FourReadsProvider {
        calls: ["first.txt", "second.txt", "third.txt", "fourth.txt"]
            .into_iter()
            .enumerate()
            .map(|(index, path)| ToolCall {
                id: format!("read-{index}"),
                name: "read".into(),
                arguments: serde_json::json!({"path": path}),
            })
            .collect(),
        turn: std::sync::Mutex::new(0),
    };
    let executor = GatedReads(tools, Arc::clone(&gate));
    let run = tokio::spawn(async move {
        run_agent_with_sink(
            &provider,
            &executor,
            request(),
            definitions,
            AgentLimits::default(),
            &AllowReadOnlyApproval,
            &NoopEventSink,
            saya_agent::CancellationToken::new(),
        )
        .await
    });
    let all_arrived = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if gate.arrivals.load(Ordering::Acquire) == 4 {
                break;
            }
            gate.all_arrived.notified().await;
        }
    })
    .await
    .is_ok();

    // Release both results on success and feature-red timeout before awaiting
    // the loop, so the assertion cannot strand either test future.
    gate.release.add_permits(4);
    let output = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .expect("the released tool loop must finish")
        .expect("the tool loop task must join")
        .expect("four contained reads and the final answer must succeed");

    assert!(
        all_arrived,
        "all four actual reads must finish dispatch before results are released"
    );
    let reads = output
        .tool_metadata
        .iter()
        .filter(|metadata| metadata.name == "read")
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 4);
    for path in ["first.txt", "second.txt", "third.txt", "fourth.txt"] {
        assert!(reads.iter().any(|read| read.arguments.contains(path)));
    }
}
