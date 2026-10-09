/// A tool executor that records every call it received.
#[derive(Default)]
struct RecordingTools {
    calls: Arc<Mutex<Vec<String>>>,
}

impl RecordingTools {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToolExecutor for RecordingTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"rows": 1}))
    }
}

/// A tool executor that seeds a numbered file into the workspace on every
/// call, so a fresh brief (rebuilt per attempt) can be told apart from a
/// carried-over one by the manifest it carries.
struct SeedingTools {
    workspace: PathBuf,
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ToolExecutor for SeedingTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let index = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(name.into());
            calls.len() - 1
        };
        fs::write(
            self.workspace.join(format!("seeded-{index}.csv")),
            format!("row {index}"),
        )
        .unwrap();
        Ok(serde_json::json!({"rows": index}))
    }
}

/// One tool definition with the given local-state effect; auto-runnable
/// (no approval, no external side effect, no database data) so the loop's
/// own gates are the only thing that can refuse it.
fn def(name: &str, local_state: LocalStateEffect) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: name.into(),
        read_only: local_state == LocalStateEffect::None,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }
}

/// One step's toolset: the executor behind it and the definitions the
/// step's episodes advertise — the composition root's per-step shape.
fn toolset(executor: Arc<dyn ToolExecutor>, definitions: Vec<ToolDefinition>) -> StepToolset {
    StepToolset {
        executor,
        definitions,
    }
}

fn call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("c-{name}"),
        name: name.into(),
        arguments: serde_json::json!({}),
    }
}
