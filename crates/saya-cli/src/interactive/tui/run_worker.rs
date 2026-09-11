//! The run worker: one fresh run driving on a background thread while the
//! TUI's event loop stays responsive — the `agent.rs` worker pattern, for
//! runs.
//!
//! The worker drives the same fresh-run path the headless `saya run` takes
//! (`commands/run/start_for_panel`); the only difference is the observers a
//! panel injects (`commands/run/host.rs`): the journal's wire forwards each
//! lifecycle event here instead of printing to a stdout the alternate screen
//! owns, the episode's agent events ride the same channel, and the
//! plan-approval ask is answered by the panel's modal — never stdin. The
//! worker captures its own `emit` output (the settle and refusal messages)
//! on the worker thread, so nothing a run prints can land on the real
//! stdout while the TUI is up.

use crate::commands::{HostRun, PlanApproval, PlanApprovalRequest};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_agent::{AgentEvent, AgentEventSink, ApprovalPolicy, CancellationToken};
use saya_store::SqliteStateStore;
use std::sync::{Arc, mpsc};
use tokio::sync::oneshot;

/// A message from the run worker to the UI.
pub(crate) enum RunMsg {
    /// A lifecycle or step event the run journal journaled — the panel
    /// shapes it with the shared `run_event_text` shaper, the one source the
    /// wire and `saya run log` share.
    Event(saya_types::RunEvent),
    /// An episode agent event; the panel's own transcript renders it.
    Episode(AgentEvent),
    /// The bound plan's approval ask (the M1-10 channel). The modal answers
    /// through `respond`; a dropped reply is a refusal, never an approval.
    PlanApproval {
        view_text: String,
        /// The plan's step goals, in plan order — the panel's step list.
        steps: Vec<String>,
        respond: oneshot::Sender<bool>,
    },
    /// The drive ended: the documented exit code plus what the run surface
    /// emitted (settle and refusal wording). Empty when the engine ended
    /// silently, because the lifecycle events already said why.
    Done(RunOutcome),
}

/// How the drive ended: the documented exit code plus the message the run
/// surface emitted for it.
pub(crate) struct RunOutcome {
    pub(crate) code: i32,
    pub(crate) message: String,
}

/// A running run the panel polls each tick: the message channel and the
/// token that stops the run.
pub(crate) struct RunWorker {
    pub(crate) rx: mpsc::Receiver<RunMsg>,
    pub(crate) cancel: CancellationToken,
}

/// The episode events' sink: forwards every agent event to the panel.
struct RunStreamSink {
    tx: mpsc::Sender<RunMsg>,
}

#[async_trait::async_trait]
impl AgentEventSink for RunStreamSink {
    async fn emit(&self, event: AgentEvent) {
        let _ = self.tx.send(RunMsg::Episode(event));
    }
}

/// The journal wire: forwards each journaled event to the panel, fired from
/// the journal's own write so the panel and the durable record are one
/// stream in one order.
fn panel_wire(tx: mpsc::Sender<RunMsg>) -> saya_harness::journal::JournalWire {
    Arc::new(move |event: &saya_types::RunEvent| {
        let _ = tx.send(RunMsg::Event(event.clone()));
    })
}

/// Relays the plan-approval asks to the panel. A panel that went away takes
/// the reply's sender with it, and the drive's `decide` reads a dropped
/// reply as a refusal — deny by default, the M1-10 rule.
async fn forward_plan_approvals(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<PlanApprovalRequest>,
    tx: mpsc::Sender<RunMsg>,
) {
    while let Some(request) = rx.recv().await {
        let PlanApprovalRequest {
            view_text,
            steps,
            respond,
        } = request;
        let goals = steps.iter().map(|step| step.goal.clone()).collect();
        let _ = tx.send(RunMsg::PlanApproval {
            view_text,
            steps: goals,
            respond,
        });
    }
}

/// Everything one panel-driven run needs, captured at start: the session's
/// runtime and store, the rendering, the approval policy and profile, and
/// the run request the `/run` tail parsed into.
pub(crate) struct RunJob {
    pub(crate) runtime: Arc<RuntimeConfig>,
    pub(crate) state_db: SqliteStateStore,
    pub(crate) format: RenderFormat,
    pub(crate) approval: ApprovalPolicy,
    pub(crate) profile: Option<String>,
    pub(crate) request: crate::commands::RunRequest,
}

/// Spawns the fresh-run worker the panel drives and returns its handle. The
/// thread owns its own current-thread runtime, exactly the way the agent
/// stream's worker does, so the event loop never blocks on the run.
pub(crate) fn spawn(job: RunJob) -> RunWorker {
    let (tx, rx) = mpsc::channel::<RunMsg>();
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let done_tx = tx.clone();
    std::thread::spawn(move || {
        // The emit seam is thread-local, so the settle and refusal messages
        // the drive prints are captured here and ride the outcome instead of
        // reaching a stdout the alternate screen owns.
        crate::commands::capture_output_start();
        let handle = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(handle) => handle,
            Err(error) => {
                let _ = crate::commands::capture_output_take();
                let _ = done_tx.send(RunMsg::Done(RunOutcome {
                    code: 5,
                    message: error.to_string(),
                }));
                return;
            }
        };
        let result = handle.block_on(drive(job, worker_cancel, tx));
        let (out, err) = crate::commands::capture_output_take();
        // A usage refusal returns a plain error the run surface never emitted
        // (the headless CLI prints it as `Error: …`, exit 2); the panel needs
        // the same words, so the error's own message rides the outcome.
        let (code, mut message) = match result {
            Ok(code) => (code, String::new()),
            Err(error) => (2, error.to_string()),
        };
        if message.is_empty() {
            message = if out.trim().is_empty() {
                err.trim().to_string()
            } else {
                out.trim().to_string()
            };
        }
        let _ = done_tx.send(RunMsg::Done(RunOutcome { code, message }));
    });
    RunWorker { rx, cancel }
}

/// The drive itself: the same fresh-run start the headless `saya run` takes,
/// with the panel's observers. Runs inside the worker's runtime.
async fn drive(
    job: RunJob,
    cancellation: CancellationToken,
    tx: mpsc::Sender<RunMsg>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let RunJob {
        runtime,
        state_db,
        format,
        approval,
        profile,
        request:
            crate::commands::RunRequest {
                run_id,
                goal,
                allow,
                budget,
            },
    } = job;
    let (approval_tx, approval_rx) = tokio::sync::mpsc::unbounded_channel::<PlanApprovalRequest>();
    tokio::spawn(forward_plan_approvals(approval_rx, tx.clone()));
    let plan_approval = PlanApproval::ViaChannel(approval_tx);
    let host = HostRun {
        journal_wire: Some(panel_wire(tx.clone())),
        agent_stream: Some(Arc::new(RunStreamSink { tx })),
        profile: profile.as_ref(),
        plan_approval: &plan_approval,
        cancellation,
    };
    crate::commands::start_for_panel(
        crate::commands::RunRequest {
            run_id,
            goal,
            allow,
            budget,
        },
        &runtime,
        format,
        approval,
        &state_db,
        host,
    )
    .await
}
