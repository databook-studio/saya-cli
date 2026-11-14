/// A run whose journal holds `events` and whose store row stands at
/// `status` — the posture a real crash leaves: the journal is the
/// authority, the store at or behind it, nothing ahead of it.
struct CrashedRun {
    root: PathBuf,
    run_dir: PathBuf,
    store: Arc<SqliteStateStore>,
    run_id: RunId,
    stubs: Stubs,
    plan: RunPlan,
    tool_calls: Arc<AtomicUsize>,
    /// One toolset per plan step, over one shared `NoTools` executor — no
    /// resume test under this rig executes a tool call.
    toolsets: Vec<StepToolset>,
}

impl CrashedRun {
    fn journal(&self) -> Vec<RunEvent> {
        Journal::open(&self.run_dir).read().unwrap()
    }

    fn inputs(&self) -> ResumeRun<'_> {
        ResumeRun {
            run_id: self.run_id.clone(),
            store: self.store.clone(),
            plan: self.plan.clone(),
            incomplete_policy: saya_harness::engine::IncompletePolicy::Conservative,
            workspace: workspace(&self.root),
            collaborators: EpisodeCollaborators {
                provider: &self.stubs.provider,
                approval: &self.stubs.approval,
                toolsets: &self.toolsets,
                cancellation: CancellationToken::default(),
            },
            request: request(),
            bounds: bounds(),
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            journal_wire: None,
            agent_stream: None,
        }
    }
}

impl Drop for CrashedRun {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Walks the store's status machine the legal way from `planned` to
/// `target` — the statuses a crash leaves behind are only ever ones the
/// store itself recorded.
async fn seed_status(store: &SqliteStateStore, id: &RunId, target: RunStatus) {
    if target == RunStatus::Planned {
        return;
    }
    store
        .set_run_status(id, RunStatus::Approved, None)
        .await
        .unwrap();
    if target != RunStatus::Approved {
        store
            .set_run_status(id, RunStatus::Executing, None)
            .await
            .unwrap();
    }
    if target == RunStatus::Paused {
        store
            .set_run_status(id, RunStatus::Paused, None)
            .await
            .unwrap();
    }
    if target == RunStatus::Completed {
        store
            .set_run_status(id, RunStatus::Completed, None)
            .await
            .unwrap();
    }
}

/// Builds a run row, a run directory with `events` appended to its journal,
/// and a store row at `status`.
async fn crashed_run(
    label: &str,
    events: &[RunEvent],
    status: RunStatus,
    plan: RunPlan,
) -> CrashedRun {
    let root = temp_root(label);
    let store = Arc::new(SqliteStateStore::new(root.join("state.sqlite3")));
    let run_id = RunId::parse(&format!("run-{label}")).unwrap();
    store
        .create_run(NewRun {
            id: run_id.clone(),
            capabilities: RunCapabilityFlags::default(),
            budgets: RunBudgets::default(),
        })
        .await
        .unwrap();
    seed_status(&store, &run_id, status).await;
    // The store's step rows mirror what the journal's step events say —
    // the journal is written before every mirror, so the store stands at
    // or behind the journal, never ahead of it.
    for event in events {
        match event {
            RunEvent::StepStarted { step } => store
                .upsert_step(&run_id, *step, RunStepStatus::Running)
                .await
                .unwrap(),
            RunEvent::StepCompleted { step } => store
                .upsert_step(&run_id, *step, RunStepStatus::Done)
                .await
                .unwrap(),
            _ => {}
        }
    }
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let journal = Journal::open(&run_dir);
    for event in events {
        journal.append(event).unwrap();
    }
    let no_tools = Arc::new(NoTools::default());
    let tool_calls = Arc::clone(&no_tools.calls);
    let executor: Arc<dyn ToolExecutor> = no_tools;
    let toolsets = (0..plan.steps.len())
        .map(|_| StepToolset {
            executor: Arc::clone(&executor),
            definitions: Vec::new(),
        })
        .collect();
    CrashedRun {
        root,
        run_dir,
        store,
        run_id,
        stubs: Stubs::default(),
        plan,
        tool_calls,
        toolsets,
    }
}
