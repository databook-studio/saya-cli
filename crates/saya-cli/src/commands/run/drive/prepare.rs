use super::super::{approval, approval_view, exit, files, tools};
use super::*;
use crate::render_run;
use crate::stream_render::TerminalSink;
use saya_harness::engine::{
    ElapsedClock, EngineEventSink, PlanDriver, PlanError, PlanRejection, PlanRequest, RunState,
    SinkBudgets, TransitionEvent, UsageTotals,
};
use saya_harness::journal::Journal;
use saya_harness::workspace::Workspace;
use saya_store::RunStore;
use std::sync::Arc;

pub(super) async fn prepare(
    inputs: DriveInputs<'_>,
) -> Result<PreparedDrive, Box<dyn std::error::Error>> {
    let DriveInputs {
        spec,
        run_dir,
        state,
        runtime,
        format,
        approval,
        allow_tokens,
        host,
    } = inputs;
    let run_id = spec.id.clone();
    let store: Arc<dyn RunStore> = Arc::new(state.clone());
    // The journal carries the host's observer when one is injected, so the
    // wire and the durable record stay one stream; the headless default
    // renders each journaled event onto the process stream in journal order.
    let journal = match host.journal_wire {
        Some(wire) => Journal::open(run_dir.root()).with_wire(wire),
        None => render_run::wired_journal(Journal::open(run_dir.root()), format),
    };
    let workspace = match Workspace::open(run_dir.workspace()) {
        Ok(workspace) => Arc::new(workspace),
        Err(error) => {
            return exit::connection_failure(
                format!("run workspace could not be opened: {error}"),
                format,
            )
            .map(PreparedDrive::Terminal);
        }
    };
    let pieces = match assembly::assemble(
        runtime,
        host.profile,
        run_dir.root(),
        &spec.scopes,
        workspace.clone(),
        approval,
        &allow_tokens,
    )
    .await
    {
        Ok(pieces) => pieces,
        Err(message) => {
            return exit::connection_failure(message, format).map(PreparedDrive::Terminal);
        }
    };
    let plan = match PlanDriver::new(
        &*pieces.provider,
        PlanRequest {
            model: pieces.model.clone(),
            run_goal: spec.goal.clone(),
        },
    )
    .propose(&pieces.plan_scopes, &spec.budgets)
    .await
    {
        Ok(plan) => plan,
        Err(PlanError::Exhausted {
            last: PlanRejection::NeedsApproval { step, scopes },
            ..
        }) => {
            // The plan asks for scopes `--allow` did not grant: the model
            // cannot fix that by re-planning, and a headless run cannot ask
            // — the refusal names the missing scopes and is a usage error
            // (2), not a provider failure.
            return crate::commands::output::failure_message(
                2,
                format!(
                    "run {run_id} cannot start: step {step} asks for {} which --allow did \
                     not grant; re-run with the missing scope(s) in --allow",
                    scopes.join(", ")
                ),
                format,
            )
            .map(PreparedDrive::Terminal);
        }
        Err(error) => {
            // The plan never bound: the run stops by cause before it began,
            // staying `planned` — there is nothing to resume, and the exit
            // says the provider layer failed.
            return crate::commands::output::failure_message(
                5,
                format!("run {run_id} could not bind a plan: {error}"),
                format,
            )
            .map(PreparedDrive::Terminal);
        }
    };
    // A step whose budget is unset inherits the run's budgets as its
    // ceilings (the StepSpec contract's layering rule); the persisted plan
    // is the layered one a resume replays.
    let plan = assembly::bind_step_budgets(plan, &spec.budgets);
    // The per-step toolsets are prebuilt from the bound plan, before the
    // approval gate: a toolset that cannot be built refuses the run before
    // it starts, never mid-flight (the same fail-closed discipline as
    // scope parsing). Each is built from its step's capabilities — the
    // thing the approval view shows — over the run's shared collaborators.
    let toolsets = tools::toolsets(
        tools::ToolsetInputs {
            database: &pieces.tools,
            scratch: pieces.scratch.as_ref(),
            fetch: pieces.fetch.as_ref(),
            runner: pieces.runner.as_ref(),
            workspace: &workspace,
            allow_query_data: pieces.allow_query_data,
            cancellation: &host.cancellation,
        },
        &plan.steps,
    );
    if let Err(error) = files::persist_plan(run_dir.root(), &plan) {
        return exit::connection_failure(
            format!("run plan could not be persisted: {error}"),
            format,
        )
        .map(PreparedDrive::Terminal);
    }
    // The approval gate at `planned → approved` (DESIGN §5.2): the plan is
    // already persisted, so a refusal or a crash here leaves a resumable
    // `planned` record — no approval was granted, and none is implied. The
    // approved scopes shown (and journaled) are the capability words plus
    // the carried grant words the run stated — every word `--allow` stated
    // is on the record a resume re-grants from.
    let view = match approval_view::view_of(
        &spec.goal,
        &spec.scopes,
        &plan,
        &spec.budgets,
        &workspace,
        assembly::manifest_bounds(),
        &super::super::scopes::carried_tokens(&allow_tokens),
    ) {
        Ok(view) => view,
        Err(message) => {
            return exit::connection_failure(message, format).map(PreparedDrive::Terminal);
        }
    };
    if !approval::decide(host.plan_approval, &view).await {
        return crate::commands::output::failure_message(
            2,
            format!(
                "run {run_id} was not approved: the plan, its scopes, and its budgets \
                 were refused at the approval gate"
            ),
            format,
        )
        .map(PreparedDrive::Terminal);
    }
    // The headless receipt (the interpreter approval's design §3): a
    // headless run's typed token is the confirmation, so the same warning
    // the interactive modal shows prints to stderr once, before the first
    // step — not a prompt, a receipt, so what the terminal shows and what
    // the journal records agree.
    if matches!(host.plan_approval, approval::PlanApproval::PreAuthorized)
        && let Some(warning) = approval_view::interpreter_warning(&view.approved)
    {
        eprintln!("{warning}");
    }
    // The clock origin is attached to the approval transition, which
    // journals PlanApproved followed by the clock before mirroring approval.
    let elapsed_clock = match spec.budgets.wall_clock {
        Some(ceiling) => match ElapsedClock::arm_now(ceiling) {
            Ok(clock) => Some(clock),
            Err(error) => {
                return exit::connection_failure(error.to_string(), format)
                    .map(PreparedDrive::Terminal);
            }
        },
        None => None,
    };
    // The sink exists from approval on: the wall-clock budget arms here,
    // never before the user has answered. The episode's events forward to
    // the host's sink when one is injected; the headless default renders
    // them through today's `TerminalEvent` envelope on the process stream.
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Planned,
        journal.clone(),
        store.clone(),
        SinkBudgets {
            wall_clock: spec.budgets.wall_clock,
            token_ceiling: super::super::budget::token_ceiling(&spec.budgets),
            // The same wallet the fetch-capable steps' executors hold: a
            // download refusal the tool reported to the model pauses the
            // run here. `None` when the run did not approve fetch — the
            // check is inert.
            download_budget: pieces.fetch.as_ref().map(|fetch| fetch.budget.clone()),
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    );
    let sink = match elapsed_clock {
        Some(clock) => sink.with_elapsed_clock(clock),
        None => sink,
    };
    let sink = sink.with_cancellation(host.cancellation.clone());
    let sink = match host.agent_stream {
        Some(stream) => sink.with_agent_stream(stream),
        None => sink.with_agent_stream(Arc::new(TerminalSink::new(format))),
    };
    if let Err(error) = sink
        .record(TransitionEvent::Approve {
            // The approved scopes as the `--allow` grammar's words — the
            // journal's durable authority a resume re-grants from, recorded
            // before anything runs.
            scopes: view.approved.clone(),
        })
        .await
    {
        return exit::connection_failure(
            format!("run approval could not be recorded: {error}"),
            format,
        )
        .map(PreparedDrive::Terminal);
    }
    Ok(PreparedDrive::Ready(Box::new(PreparedState {
        run_id,
        format,
        pieces,
        workspace,
        plan,
        toolsets,
        sink,
        store,
        journal,
        cancellation: host.cancellation,
    })))
}
