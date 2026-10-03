//! `saya run "<goal>"` — the fresh-run entry point.
//!
//! Refuses by construction before anything exists on disk: scopes unstated,
//! no run. `--allow none` is the stated empty approval — a read-only run,
//! since the episode's per-tool-call decider already defaults to read-only
//! (`assembly.rs`) and nothing else is reachable. Then claims the run,
//! proposes and binds a plan, and drives every step blocking. Ctrl-C
//! cancels and exits 130, the pattern `commands/query.rs` uses; every typed
//! outcome lands in the shared exit mapping (`drive.rs`).
//!
//! A host that owns the terminal — the TUI's run panel — starts its run
//! through [`start_for_panel`]: the same parsing, refusal checks, claim, and
//! drive, with the host's observers (`commands/run/host.rs`) and its own
//! token. That token is the cover Ctrl-C is headless; a stop the drive's
//! engine never recorded itself lands in [`super::cancel::record_cancelled`]
//! — the same path `saya run cancel` takes — so the panel's cancel and the
//! CLI's cancel write one durable record.

use super::approval;
use super::drive;
use super::host::{HostRun, RunRequest};
use super::mode::RunApproval;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use prepare::prepare;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::SqliteStateStore;

mod prepare;

/// The fresh-run invocation inputs: the goal prompt, the `--allow` scopes,
/// and the budget overrides. The host entry passes the same three; only the
/// headless path adds `can_prompt`.
pub(super) struct StartInputs<'a> {
    pub(super) prompt: Option<String>,
    pub(super) allow: &'a [String],
    pub(super) budget_tokens: &'a [String],
    pub(super) can_prompt: bool,
}

pub(super) async fn start(
    inputs: StartInputs<'_>,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    // The entry guard, shared with every other entry point into a run
    // (`mode.rs`): a run's approval is its `--allow` scopes; bypass is a
    // session mode. Refused before anything exists on disk.
    let approval = RunApproval::admit(approval)?;
    let StartInputs {
        prompt,
        allow,
        budget_tokens,
        can_prompt,
    } = inputs;
    let cancellation = CancellationToken::default();
    let prepared = match prepare(
        &RunRequest {
            run_id: super::new_run_id(),
            goal: prompt,
            allow: allow.to_vec(),
            budget: budget_tokens.to_vec(),
        },
        runtime,
        format,
        state,
        None,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(outcome) => return outcome,
    };
    // The plan-approval surface: a run that can interact asks once over the
    // channel (the `tui/agent.rs` pattern, the terminal answering); a
    // headless run's approval is the RunSpec pre-authorization — `--allow`
    // declared the scopes, and the engine refuses any plan outside them.
    let plan_approval = if can_prompt {
        super::ask::terminal()
    } else {
        approval::PlanApproval::PreAuthorized
    };
    let spec = prepared.spec;
    let run_dir = prepared.run_dir.clone();
    let host = super::host::HostRun {
        journal_wire: None,
        agent_stream: None,
        profile: None,
        plan_approval: &plan_approval,
        cancellation: cancellation.clone(),
    };
    let work = drive::drive(drive::DriveInputs {
        spec: &spec,
        run_dir,
        state,
        runtime,
        format,
        approval,
        allow_tokens: prepared.allow_tokens,
        host,
    });
    tokio::pin!(work);
    // Ctrl-C covers everything below: the plan proposal, the approval, and
    // every episode. Once signalled, keep the drive future and run lock alive
    // until its current source future settles.
    let stopped_by_ctrl_c = tokio::select! {
        result = &mut work => result,
        _ = tokio::signal::ctrl_c() => {
            cancellation.cancel();
            work.await
        }
    };
    if cancellation.reason() == Some(saya_agent::CancellationReason::User) {
        match super::cancel::record_cancelled(&spec.id, prepared.run_dir.root(), state, None).await
        {
            super::cancel::CancelOutcome::Recorded => {
                drop(prepared.lock);
                return Ok(130);
            }
            super::cancel::CancelOutcome::AlreadyFinished(_) => {}
            _ => {}
        }
    }
    match stopped_by_ctrl_c {
        Ok(exit) => {
            drop(prepared.lock);
            Ok(exit)
        }
        Err(error) => Err(error),
    }
}

/// The host path a TUI run panel drives: the same fresh-run start with the
/// panel's observers, covered by the panel's token instead of Ctrl-C. When
/// the token stops a run the drive never got to record — before approval, or
/// wherever the engine's own cancel lost the race — the stop is recorded
/// through the same engine path `saya run cancel` takes, so the journal and
/// the store end in the state that command would have left.
pub(crate) async fn start_for_panel(
    request: RunRequest,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    state: &SqliteStateStore,
    host: HostRun<'_>,
) -> Result<i32, Box<dyn std::error::Error>> {
    // The entry guard, shared with every other entry point into a run
    // (`mode.rs`): a run's approval is its `--allow` scopes; bypass is a
    // session mode. Refused before anything exists on disk.
    let approval = RunApproval::admit(approval)?;
    let token = host.cancellation.clone();
    let wire = host.journal_wire.clone();
    let prepared = match prepare(&request, runtime, format, state, wire.clone()).await {
        Ok(prepared) => prepared,
        Err(outcome) => return outcome,
    };
    let run_id = prepared.spec.id.clone();
    let spec = prepared.spec;
    let work = drive::drive(drive::DriveInputs {
        spec: &spec,
        run_dir: prepared.run_dir.clone(),
        state,
        runtime,
        format,
        approval,
        allow_tokens: prepared.allow_tokens,
        host,
    });
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => {
            drop(prepared.lock);
            return result;
        }
        _ = token.cancelled() => {}
    }
    // Do not release the run lock while an episode or source future is still
    // owned by the drive. If cancellation arrived before the engine acquired
    // a lifecycle sink, settle the durable fallback only after that drive
    // future has returned.
    let result = work.await;
    if token.reason() != Some(saya_agent::CancellationReason::User) {
        drop(prepared.lock);
        return result;
    }
    let outcome =
        super::cancel::record_cancelled(&run_id, prepared.run_dir.root(), state, wire).await;
    drop(prepared.lock);
    match outcome {
        super::cancel::CancelOutcome::Recorded => Ok(130),
        super::cancel::CancelOutcome::AlreadyFinished(
            saya_harness::engine::RunState::Cancelled | saya_harness::engine::RunState::Paused,
        ) => result,
        _ => result,
    }
}
