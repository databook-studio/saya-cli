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
use super::host::{HostRun, RunRequest};
use super::{budget, claim, drive};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_harness::journal::JournalWire;
use saya_harness::lock::RunLock;
use saya_harness::run_dir::RunDir;
use saya_store::SqliteStateStore;
use saya_types::RunSpec;

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
        approval,
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
    let host = super::host::HostRun {
        journal_wire: None,
        agent_stream: None,
        profile: None,
        plan_approval: &plan_approval,
        cancellation: cancellation.clone(),
    };
    let work = drive::drive(drive::DriveInputs {
        spec: &spec,
        run_dir: prepared.run_dir,
        state,
        runtime,
        format,
        approval,
        host,
    });
    tokio::pin!(work);
    // Ctrl-C covers everything below: the plan proposal, the approval, and
    // every episode.
    match tokio::select! {
        result = &mut work => result,
        _ = tokio::signal::ctrl_c() => { cancellation.cancel(); return Ok(130); }
    } {
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
    let token = host.cancellation.clone();
    let wire = host.journal_wire.clone();
    let prepared = match prepare(&request, runtime, format, approval, state, wire.clone()).await {
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
    // The engine records the stop itself once an episode observes the token;
    // a stop it never saw (mid-plan, mid-approval) is recorded here — the
    // same machine, journal, and store mirror `saya run cancel` uses, with
    // the host's wire so the panel sees the same event the journal wrote. An
    // already-terminal tail (the drive recorded it while this arm raced it)
    // leaves the record untouched: exactly one `Cancelled`, either way.
    super::cancel::record_cancelled(&run_id, prepared.run_dir.root(), state, wire).await;
    drop(prepared.lock);
    Ok(130)
}

/// Parses and refuses before anything exists on disk, then claims the run.
/// The error arm is the *mapped outcome* the caller returns: usage refusals
/// surface as plain errors (exit 2 through the app), a claim failure as the
/// connection class (3) with its message already emitted.
async fn prepare(
    request: &RunRequest,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    state: &SqliteStateStore,
    wire: Option<JournalWire>,
) -> Result<Prepared, Result<i32, Box<dyn std::error::Error>>> {
    let goal = request
        .goal
        .clone()
        .map(|text| text.trim().to_string())
        .ok_or(Err(
            "run requires a goal: `saya run \"<goal>\" --allow <scopes>`".into(),
        ))?;
    // A run's approval is its `--allow` scopes — typed, per-capability,
    // journaled. Bypass is a session mode: a blanket per-call consent, and a
    // run has no per-call consent to replace. Refused at start, beside the
    // missing-`--allow` refusal, before anything exists on disk.
    if let Err(error) = refuse_bypass_mode(approval) {
        return Err(Err(error));
    }
    // Refusal by construction, before anything exists on disk: a headless run
    // states its scopes up front or does not start — no run directory, no
    // store row, no prompt. The refusal is about *stating*: `--allow none`
    // states the empty approval and starts a read-only run, so only an
    // absent `--allow` refuses here.
    if request.allow.is_empty() {
        return Err(Err(
            "a headless run refuses to start without --allow <scopes>: it cannot \
             prompt for approval mid-run, so its scopes must be declared up front"
                .into(),
        ));
    }
    let approved = super::scopes::parse(&request.allow, super::scopes::Surface::Run)
        .map_err(|message| Err(message.into()))?;
    let budgets = budget::parse(&request.budget, &runtime.resolved.jobs.budgets())
        .map_err(|message| Err(message.into()))?;
    let spec = RunSpec::new(
        request.run_id.clone(),
        &goal,
        approved.capabilities.clone(),
        budgets.clone(),
    )
    .map_err(|error| Err(format!("run spec refused: {error}").into()))?;
    let (run_dir, lock) = match claim::claim(&request.run_id, state, &spec, format, wire).await {
        Ok(claimed) => claimed,
        Err(message) => return Err(claim::claim_failure(message, format)),
    };
    Ok(Prepared {
        spec,
        run_dir,
        lock,
    })
}

/// The run-start guard for a bypass mode: a run's approval is its `--allow`
/// scopes; bypass is a session mode. Pure, so the test can drive the exact
/// refusal both fresh-run entries share.
pub(super) fn refuse_bypass_mode(
    approval: ApprovalPolicy,
) -> Result<(), Box<dyn std::error::Error>> {
    if approval == ApprovalPolicy::Bypass {
        return Err("a run's approval is its `--allow` scopes; bypass is a session mode".into());
    }
    Ok(())
}

/// What a claim leaves the caller holding: the spec the drive reads, the
/// claimed directory, and the single-writer lock the run holds for its life.
struct Prepared {
    spec: RunSpec,
    run_dir: RunDir,
    lock: RunLock,
}
