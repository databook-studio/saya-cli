fn step(goal: &str, capabilities: Capabilities) -> StepSpec {
    StepSpec::new(goal, capabilities, None, Vec::new(), None).unwrap()
}

fn step_with_budget(goal: &str, budget: Option<Budgets>) -> StepSpec {
    StepSpec::new(goal, Capabilities::default(), budget, Vec::new(), None).unwrap()
}

fn bounds() -> ManifestBounds {
    ManifestBounds {
        max_files: 8,
        max_file_bytes: 64 * 1024,
    }
}

/// Auto-approves everything: the stub tools declare no approval requirement,
/// so the episodes' runs need no interactive decision.
struct AllowApproval;

#[async_trait]
impl ApprovalDecider for AllowApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

fn driver_and_sink<'a>(
    run: &ApprovedRun,
    provider: &'a ScriptProvider,
    approval: &'a AllowApproval,
    toolsets: &'a [StepToolset],
    memory_allows_candidate_writes: bool,
) -> (EngineEventSink, EpisodeDriver<'a>) {
    let cancellation = CancellationToken::default();
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        Journal::open(&run.run_dir),
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    )
    .with_cancellation(cancellation.clone());
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider,
            approval,
            toolsets,
            cancellation,
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: Journal::open(&run.run_dir),
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes,
        },
        bounds(),
    );
    (sink, driver)
}

/// Every message's content of one captured request, joined — the brief is
/// in there and this is how tests read it.
fn texts(request: &ChatRequest) -> String {
    request
        .messages
        .iter()
        .map(|message| message.content.clone())
        .collect::<Vec<_>>()
        .join("\n")
}
