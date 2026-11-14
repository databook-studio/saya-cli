use super::super::exit::Settled;
use super::super::{assembly, exit};
use super::{PreparedDrive, PreparedState};
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeRequest, EpisodeRun,
};

pub(super) async fn execute(prepared: PreparedDrive) -> Result<i32, Box<dyn std::error::Error>> {
    let PreparedState {
        run_id,
        format,
        pieces,
        workspace,
        plan,
        toolsets,
        sink,
        store,
        journal,
        cancellation,
    } = match prepared {
        PreparedDrive::Terminal(code) => return Ok(code),
        PreparedDrive::Ready(state) => *state,
    };
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &*pieces.provider,
            approval: &pieces.decider,
            toolsets: &toolsets,
            cancellation: cancellation.clone(),
        },
        EpisodeRun {
            run_id: run_id.clone(),
            store: store.clone(),
            journal,
        },
        EpisodeRequest {
            model: pieces.model.clone(),
            profile_names: pieces.profile_names,
            memory_allows_candidate_writes: false,
        },
        assembly::manifest_bounds(),
    );
    match drive_steps(&driver, &sink, &plan, &workspace).await {
        Ok(()) => exit::settle(
            Settled {
                state: sink.state(),
                code: None,
            },
            &run_id,
            format,
        ),
        Err(error @ saya_harness::engine::EpisodeError::OwnerFailureDiagnostic { .. })
        | Err(error @ saya_harness::engine::EpisodeError::SinkDiagnostic { .. }) => {
            crate::commands::output::failure_message(
                3,
                format!("run {run_id} stopped after an owner error: {error}"),
                format,
            )
        }
        Err(error @ saya_harness::engine::EpisodeError::BudgetStoppedDiagnostic { .. }) => {
            crate::commands::output::failure_message(
                6,
                format!("run {run_id} paused after its stop settled: {error}"),
                format,
            )
        }
        Err(
            error @ (saya_harness::engine::EpisodeError::CancelledDiagnostic { .. }
            | saya_harness::engine::EpisodeError::CancelPersistenceFailed { .. }
            | saya_harness::engine::EpisodeError::BudgetPersistenceFailed { .. }
            | saya_harness::engine::EpisodeError::Transition { .. }),
        ) if matches!(
            sink.state(),
            saya_harness::engine::RunState::Completed
                | saya_harness::engine::RunState::Failed
                | saya_harness::engine::RunState::Cancelled
                | saya_harness::engine::RunState::Paused
        ) =>
        {
            exit::settle_diagnostic(
                Settled {
                    state: sink.state(),
                    code: exit::failure_code_of(&error),
                },
                &error,
                &run_id,
                format,
            )
        }
        Err(error)
            if matches!(
                sink.state(),
                saya_harness::engine::RunState::Planned
                    | saya_harness::engine::RunState::Approved
                    | saya_harness::engine::RunState::Executing
            ) =>
        {
            crate::commands::output::failure_message(
                3,
                format!("run {run_id} could not settle: {error}"),
                format,
            )
        }
        Err(error) => exit::settle(
            Settled {
                state: sink.state(),
                code: exit::failure_code_of(&error),
            },
            &run_id,
            format,
        ),
    }
}

/// Drives the plan's steps in order; the first error stops the loop.
async fn drive_steps(
    driver: &EpisodeDriver<'_>,
    sink: &EngineEventSink,
    plan: &saya_types::RunPlan,
    workspace: &saya_harness::workspace::Workspace,
) -> Result<(), saya_harness::engine::EpisodeError> {
    for step in 0..plan.steps.len() {
        driver.run_step(sink, plan, step, workspace).await?;
    }
    Ok(())
}
