use super::*;

/// The one that matters for the wallet: the download budget binds the
/// **run**, not each invocation. A run pauses on its 100-byte wallet having
/// claimed 97; a resume seeds the wallet the composition armed with the
/// spend the journal records, so the resumed download's 40-byte claim is
/// refused — a fresh wallet would have accepted it — and the refusal, the
/// same latch, pauses the run again. Under the re-arm behaviour this
/// replaces, the resumed invocation would have started from zero and spent
/// a whole fresh wallet.
#[tokio::test]
async fn a_download_paused_run_resumed_without_a_fresh_wallet_continues_against_its_recorded_spend()
{
    let plan = RunPlan::new(vec![step("download the corpus")]).unwrap();
    let run = crashed_run(
        "download-resume",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::DownloadedBytes { bytes: 60 },
            RunEvent::DownloadedBytes { bytes: 97 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted,
            },
        ],
        RunStatus::Paused,
        plan,
    )
    .await;
    let wallet = DownloadBudget::new(100);
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![tool_call("http_download")]),
        Turn::Answer("done"),
    ]);
    let executor: Arc<dyn ToolExecutor> = Arc::new(Claiming {
        wallet: wallet.clone(),
        bytes: 40,
    });
    let toolsets = vec![StepToolset {
        executor,
        definitions: vec![download_definition()],
    }];

    let outcome = resume(
        &run.run_dir,
        download_inputs(&run, &provider, &toolsets, wallet.clone()),
    )
    .await;

    // The machine refuses the paused run's completion — the same shape
    // every mid-episode pause leaves.
    let error = outcome.unwrap_err();
    assert!(
        matches!(error, ResumeError::Episode { .. }),
        "the machine must refuse a paused run's completion: {error:?}"
    );

    // The wallet binds the run: the claimed 40 were refused against the 97
    // the record holds, the refusal tripped the latch, and the run stands
    // paused — not running on a fresh wallet.
    assert_eq!(
        wallet.consumed(),
        97,
        "the run's budget is spent, not a fresh one"
    );
    assert!(wallet.tripped(), "the refusal is the recorded event");
    let recorded = run.store.get_run(&run.run_id).await.unwrap().unwrap();
    assert_eq!(recorded.status, RunStatus::Paused);

    // The journal: the first invocation's record, then the resumed attempt
    // started and settled at the first refused claim, with no step completion.
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::DownloadedBytes { bytes: 60 },
            RunEvent::DownloadedBytes { bytes: 97 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
            RunEvent::StepStarted { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
        ],
        "the resume must continue against the recorded spend, not a fresh wallet"
    );
}

/// The carried headroom is real, and the resumed claims record the level
/// they reached: the resumed download claims the three bytes the record
/// leaves (97 of 100), the sink journals the level — 100, the carried 97
/// plus this invocation's 3, never the claim's own 3 — and the next claim
/// past the limit is refused, tripping the latch and pausing the run. The
/// journaled level is what pins the whole chain: without the carry it would
/// read 3, and a fresh baseline would have re-journaled the record's 97.
#[tokio::test]
async fn a_resumed_download_claims_the_carried_headroom_and_records_the_level_it_reached() {
    let plan = RunPlan::new(vec![step("download the corpus")]).unwrap();
    let run = crashed_run(
        "download-headroom",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::DownloadedBytes { bytes: 60 },
            RunEvent::DownloadedBytes { bytes: 97 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted,
            },
        ],
        RunStatus::Paused,
        plan,
    )
    .await;
    let wallet = DownloadBudget::new(100);
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![tool_call("http_download")]),
        Turn::Tools(vec![tool_call("http_download")]),
        Turn::Answer("done"),
    ]);
    let executor: Arc<dyn ToolExecutor> = Arc::new(Claiming {
        wallet: wallet.clone(),
        bytes: 3,
    });
    let toolsets = vec![StepToolset {
        executor,
        definitions: vec![download_definition()],
    }];

    let outcome = resume(
        &run.run_dir,
        download_inputs(&run, &provider, &toolsets, wallet.clone()),
    )
    .await;

    let error = outcome.unwrap_err();
    assert!(
        matches!(error, ResumeError::Episode { .. }),
        "the machine must refuse a paused run's completion: {error:?}"
    );

    // The headroom claim fit and was spent; the claim past the limit was
    // refused and tripped the latch — the run's budget, not a fresh one.
    assert_eq!(wallet.consumed(), 100, "the headroom the record leaves");
    assert!(wallet.tripped(), "the claim past the limit is the refusal");
    let recorded = run.store.get_run(&run.run_id).await.unwrap().unwrap();
    assert_eq!(recorded.status, RunStatus::Paused);

    // The journal: the resume journaled the level the wallet reached —
    // 100, cumulative — and re-journaled nothing the record already held.
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::DownloadedBytes { bytes: 60 },
            RunEvent::DownloadedBytes { bytes: 97 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
            RunEvent::StepStarted { step: 0 },
            RunEvent::DownloadedBytes { bytes: 100 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
        ],
        "the resumed claims must record the level they reached, past the \
         carried spend, never the carried spend again"
    );
}
