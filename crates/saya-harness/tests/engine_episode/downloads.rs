use super::*;

/// The fetch scope's end-to-end gate (S2 decision 2, test 5): a step whose
/// capabilities asked for fetch — and only fetch — downloads through
/// `http_download` in a real episode; the tiny run budget refuses a chunk,
/// the tool's typed error reaches the model, the trip latch pauses the run
/// with the shared `BudgetExhausted` reason, and the next `run_step`
/// refuses on state. Paused, not overrun — end to end.
#[tokio::test]
async fn a_download_that_trips_the_run_budget_pauses_the_run_typed() {
    use saya_harness::fetch::{
        DownloadBudget, DownloadLimits, FetchLimits, FetchPolicy, FetchTools,
    };
    use saya_types::{Destination, FetchScope};

    let run = approved_run("fetch-budget").await;
    let workspace = Arc::new(run.workspace());
    let mut fetch_caps = Capabilities::default();
    fetch_caps.fetch = Some(
        FetchScope::new(vec![
            Destination::new("https", "files.example.org").unwrap(),
        ])
        .expect("shaped"),
    );
    let budget = DownloadBudget::new(8);
    let tools = FetchTools::new(
        FetchPolicy::new(vec![FetchDestination::new("https", "files.example.org")]),
        Arc::new(FetchNet {
            chunks: VecDeque::from(vec![vec![b'x'; 4], vec![b'x'; 4], vec![b'x'; 40]]),
        }),
        FetchLimits::for_tool_lane(),
        DownloadLimits::default(),
        Some(Arc::clone(&workspace)),
        budget.clone(),
    );
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![ToolCall {
            id: "c-download".into(),
            name: "http_download".into(),
            arguments: serde_json::json!({
                "url": "https://files.example.org/corpus.bin",
                "destination": "downloads/corpus.bin"
            }),
        }]),
        Turn::Answer("done"),
    ]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(tools),
        vec![saya_harness::fetch::http_download_definition()],
    )];
    // The sink arms the same wallet the step's member holds: a download
    // refusal the model has already seen as a typed error also pauses the
    // run here — clone-shares-state.
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        Journal::open(&run.run_dir),
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: Some(budget.clone()),
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    );
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &provider,
            approval: &approval,
            toolsets: &toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: Journal::open(&run.run_dir),
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        bounds(),
    );
    let plan = RunPlan::new(vec![
        step("pull the corpus", fetch_caps.clone()),
        step("read only", Capabilities::default()),
    ])
    .unwrap();

    // Step 0 runs: the download claims 8 bytes, the next chunk is refused
    // typed, the episode ends informed, and the tick after the trip pauses
    // the run.
    driver
        .run_step(&sink, &plan, 0, workspace.as_ref())
        .await
        .expect("the episode itself completes; the pause is level-triggered");
    let second_request = &provider.requests()[1];
    let tool_text = texts(second_request);
    assert!(
        tool_text.contains("the run's download budget of 8 bytes tripped after 8"),
        "the model must see the typed budget error, never a short success: {tool_text}"
    );
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "the trip latch pauses the run on the first tick after the trip"
    );
    assert!(budget.tripped(), "the refusal is the recorded event");
    let on_disk = walk_dir_bytes(&run.root.join("workspace"));
    assert!(on_disk <= 8, "paused, not overrun: {on_disk} bytes on disk");

    // The next step refuses on state — a paused run is not runnable.
    let error = driver
        .run_step(&sink, &plan, 1, workspace.as_ref())
        .await
        .expect_err("a paused run cannot begin its next step");
    assert!(
        matches!(
            error,
            EpisodeError::NotRunnable {
                state: RunState::Paused,
                ..
            }
        ),
        "the step boundary stops the run: {error:?}"
    );

    // The durable record names the reason — the shared vocabulary, no new
    // pause reason.
    let events = Journal::open(&run.run_dir).read().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            event,
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            }
        )),
        "the pause must name the budget: {events:?}"
    );

    let _ = fs::remove_dir_all(run.root);
}
