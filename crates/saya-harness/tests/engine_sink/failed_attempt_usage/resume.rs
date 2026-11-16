//! The resume leg: the run's journaled 44 carried tokens stand against the
//! 40-token ceiling the moment a retry resumes — the pause is journaled
//! `BudgetExhausted` without a fabricated receipt, and the sink ends Paused.

use std::path::Path;
use std::sync::Arc;

use saya_agent::CancellationToken;
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeRequest, IncompletePolicy, ManifestBounds,
    ResumeError, ResumeRun, RunState, resume,
};
use saya_harness::journal::Journal;
use saya_harness::workspace::Workspace;
use saya_store::SqliteStateStore;
use saya_types::{Capabilities, RunEvent, RunId, RunPlan, StepSpec};

use super::fixtures::{NoTools, ScriptedProvider};

pub(super) async fn resume_pauses_on_carried_tokens(
    root: &Path,
    run_dir: &Path,
    journal: &Journal,
    run_id: &RunId,
    store: Arc<SqliteStateStore>,
    provider: &ScriptedProvider,
    sink: &EngineEventSink,
) {
    let step = StepSpec::new(
        "retry bounded work",
        Capabilities::default(),
        None,
        Vec::new(),
        None,
    )
    .unwrap();
    let plan = RunPlan::new(vec![step]).unwrap();
    let toolsets = vec![saya_harness::engine::StepToolset {
        executor: Arc::new(NoTools::default()),
        definitions: Vec::new(),
    }];
    let workspace_dir = root.join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    let resumed = resume(
        run_dir,
        ResumeRun {
            run_id: run_id.clone(),
            store,
            plan,
            incomplete_policy: IncompletePolicy::Retry,
            workspace: Workspace::open(&workspace_dir).unwrap(),
            collaborators: EpisodeCollaborators {
                provider,
                approval: &saya_agent::AllowReadOnlyApproval,
                toolsets: &toolsets,
                cancellation: CancellationToken::default(),
            },
            request: EpisodeRequest {
                model: "scripted".into(),
                profile_names: Vec::new(),
                memory_allows_candidate_writes: false,
            },
            bounds: ManifestBounds {
                max_files: 8,
                max_file_bytes: 64 * 1024,
            },
            wall_clock: None,
            token_ceiling: Some(40),
            download_budget: None,
            journal_wire: None,
            agent_stream: None,
        },
    )
    .await;
    assert!(matches!(resumed, Err(ResumeError::Episode { .. })));
    let events = journal.read().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                RunEvent::Paused {
                    reason: saya_types::PauseReason::BudgetExhausted
                }
            ))
            .count(),
        2,
        "resume carries the known 44 past the 40-token ceiling: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::Usage { .. }))
            .count(),
        2,
        "resume checks the carried total without fabricating another receipt"
    );
    assert_eq!(sink.state(), RunState::Paused);
}
