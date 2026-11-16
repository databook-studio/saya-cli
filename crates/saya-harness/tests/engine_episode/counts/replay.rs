use super::*;

fn usage_pair(turns: Option<u64>, tool_calls: Option<u64>) -> RunEvent {
    RunEvent::Usage {
        endpoint: "orchestrator".into(),
        tokens: None,
        turns,
        tool_calls,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

#[tokio::test]
async fn the_settled_journal_replays_not_pending_with_requests_carried() {
    use saya_harness::engine::{IncompletePolicy, ResumeError, ResumeRun, resume};

    let run = approved_run("count-replay").await;
    run.store
        .set_run_status(&run.run_id, RunStatus::Executing, None)
        .await
        .unwrap();
    let journal = Journal::open(&run.run_dir);
    for event in [
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
        RunEvent::StepStarted { step: 0 },
        usage_pair(Some(0), Some(0)),
        usage_pair(Some(1), None),
        usage_pair(None, Some(0)),
        RunEvent::StepFailed { step: 0 },
        RunEvent::StepStarted { step: 0 },
        usage_pair(Some(1), None),
        usage_pair(None, Some(0)),
    ] {
        journal.append(&event).unwrap();
    }
    let provider = ScriptProvider::new(vec![Turn::Answer("must not be admitted")]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let mut budget = Budgets::default();
    budget.turns = Some(2);
    let plan = RunPlan::new(vec![step_with_budget("the settled step", Some(budget))]).unwrap();

    let error = resume(
        &run.run_dir,
        ResumeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            plan,
            incomplete_policy: IncompletePolicy::Retry,
            workspace: run.workspace(),
            collaborators: EpisodeCollaborators {
                provider: &provider,
                approval: &approval,
                toolsets: &toolsets,
                cancellation: CancellationToken::default(),
            },
            request: EpisodeRequest {
                model: "mock-model".into(),
                profile_names: Vec::new(),
                memory_allows_candidate_writes: false,
            },
            bounds: bounds(),
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            journal_wire: None,
            agent_stream: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        !matches!(error, ResumeError::CountEvidence { .. }),
        "the settled journal replays not-pending, never a count refusal: {error:?}"
    );
    assert!(
        matches!(
            error,
            ResumeError::Episode {
                source: EpisodeError::BudgetStopped { .. }
            }
        ),
        "the run continued on the replayed counts and stopped on its spent grant: {error:?}"
    );
    assert!(
        provider.requests().is_empty(),
        "the journal carried requests == 2: the turns=2 grant admits nothing more"
    );
    let _ = fs::remove_dir_all(run.root);
}
