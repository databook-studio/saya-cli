//! Public-boundary scheduling probes for explicit tool concurrency metadata.

use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, LocalStateEffect, ProviderError, ToolCall, ToolConcurrency, ToolDefinition,
    ToolEffect, ToolError, ToolExecutor, run_agent,
};
use std::sync::{Arc, Mutex};

struct ProbeProvider {
    responses: Mutex<Vec<ChatResponse>>,
}

#[async_trait]
impl ChatProvider for ProbeProvider {
    fn name(&self) -> &str {
        "concurrency-probe"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Ok(self.responses.lock().unwrap().remove(0))
    }
}

struct ConcurrencyProbe {
    started: std::sync::atomic::AtomicUsize,
    started_notify: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
    seen: Mutex<Vec<String>>,
}

#[async_trait]
impl ToolExecutor for ConcurrencyProbe {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.seen.lock().unwrap().push(name.into());
        self.started
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.started_notify.notify_waiters();
        let _permit = self
            .release
            .acquire()
            .await
            .expect("probe gate remains open");
        Ok(serde_json::json!({"name": name}))
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "test concurrency".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn probe_tool(name: &str, read_only: bool, concurrency: ToolConcurrency) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: "concurrency probe".into(),
        read_only,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency,
        completion: None,
    }
}

fn probe_provider(calls: Vec<ToolCall>) -> ProbeProvider {
    ProbeProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: calls,
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "done")),
        ]),
    }
}

async fn wait_for_probe_starts(probe: &ConcurrencyProbe, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let notified = probe.started_notify.notified();
            if probe.started.load(std::sync::atomic::Ordering::SeqCst) >= expected {
                return;
            }
            notified.await;
        }
    })
    .await
    .expect("tool calls must start before the timeout guard");
}

fn calls(definitions: &[ToolDefinition]) -> Vec<ToolCall> {
    definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| ToolCall {
            id: format!("call-{index}"),
            name: definition.name.clone(),
            arguments: serde_json::json!({}),
        })
        .collect()
}

/// Tool metadata is serial by default: neither a read-shaped declaration nor
/// a policy allowance grants concurrent execution.
#[tokio::test]
async fn default_serial_tool_calls_never_overlap() {
    let probe = Arc::new(ConcurrencyProbe {
        started: std::sync::atomic::AtomicUsize::new(0),
        started_notify: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let mut legacy = serde_json::to_value(probe_tool("first", true, ToolConcurrency::Serial))
        .expect("probe definition serializes");
    legacy
        .as_object_mut()
        .expect("definition is an object")
        .remove("concurrency");
    let first: ToolDefinition =
        serde_json::from_value(legacy).expect("legacy definition deserializes");
    assert_eq!(first.concurrency, ToolConcurrency::Serial);
    let definitions = vec![first, probe_tool("second", true, ToolConcurrency::Serial)];
    let provider = probe_provider(calls(&definitions));
    let run = run_agent(
        &provider,
        probe.as_ref(),
        request(),
        definitions,
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    );
    tokio::pin!(run);
    tokio::select! {
        output = &mut run => panic!("run completed before its first probe call: {output:?}"),
        _ = wait_for_probe_starts(&probe, 1) => {}
    }
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            wait_for_probe_starts(&probe, 2)
        )
        .await
        .is_err(),
        "a default-serial second call must wait for the first to settle"
    );
    probe.release.add_permits(2);
    assert_eq!(run.await.unwrap().answer, "done");
}

/// Explicitly concurrent, approval-free reads can overlap; the fixed cap is
/// exercised by the existing twelve-call public-boundary test.
#[tokio::test]
async fn explicitly_concurrent_reads_overlap() {
    let probe = Arc::new(ConcurrencyProbe {
        started: std::sync::atomic::AtomicUsize::new(0),
        started_notify: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let definitions = vec![
        probe_tool("first", true, ToolConcurrency::Concurrent),
        probe_tool("second", true, ToolConcurrency::Concurrent),
    ];
    let provider = probe_provider(calls(&definitions));
    let run = run_agent(
        &provider,
        probe.as_ref(),
        request(),
        definitions,
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    );
    tokio::pin!(run);
    tokio::select! {
        output = &mut run => panic!("run completed before concurrent calls started: {output:?}"),
        _ = wait_for_probe_starts(&probe, 2) => {}
    }
    probe.release.add_permits(2);
    assert_eq!(run.await.unwrap().answer, "done");
}

/// A permitted write and a serial barrier both keep the whole mixed batch in
/// input order; no metadata is inferred from the write declaration itself.
#[tokio::test]
async fn permitted_serial_write_and_mixed_barrier_keep_input_order() {
    let probe = Arc::new(ConcurrencyProbe {
        started: std::sync::atomic::AtomicUsize::new(0),
        started_notify: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let definitions = vec![
        probe_tool("before", true, ToolConcurrency::Concurrent),
        probe_tool("write", false, ToolConcurrency::Serial),
        probe_tool("after", true, ToolConcurrency::Concurrent),
    ];
    let provider = probe_provider(calls(&definitions));
    let run = run_agent(
        &provider,
        probe.as_ref(),
        request(),
        definitions,
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    );
    tokio::pin!(run);
    tokio::select! {
        output = &mut run => panic!("run completed before its first probe call: {output:?}"),
        _ = wait_for_probe_starts(&probe, 1) => {}
    }
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            wait_for_probe_starts(&probe, 2)
        )
        .await
        .is_err(),
        "a serial member makes the mixed batch conservative"
    );
    probe.release.add_permits(3);
    assert_eq!(run.await.unwrap().answer, "done");
    assert_eq!(&*probe.seen.lock().unwrap(), &["before", "write", "after"]);
}
