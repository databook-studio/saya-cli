//! The surface the launch opens, and the wiring the loop is started with.
//!
//! The session is composed over the surface's own front, so the front exists
//! before the loop does: this module states the pieces the launch hands over —
//! the queue and its producers, the front the session talks to, the session
//! itself, the operations its commands state and what its direct commands need
//! — and starts the loop on the real terminal.
//!
//! Everything here is the launch's, not the loop's: the loop below it is a
//! function of its queue ([`super::run`]).

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use saya_agent::{AgentMetric, AgentMetricOutcome, AgentMetricPhase};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use super::direct_run::DirectDeps;
use super::jobs_open::JobDeps;
use super::live::LiveRuntime;
use super::model::Model;
use super::msg::Msg;
use super::ops::OpsDeps;
use super::run::Runtime;
use super::run_clock::SystemClock;
use super::run_cmd_port::{PortParts, SessionPort};
use super::run_draw::Facts;
use super::run_front::Forward;
use super::run_input::{self, InputGate};
use super::run_screen::TtyScreen;
use crate::chat::{ChatSession, OpsContext};
use crate::cli::GlobalOptions;
use crate::interactive::SessionState;
use crate::local_metrics::LocalMetrics;

/// The surface's queue: the producers' end, and the loop's.
pub(crate) fn channel() -> (UnboundedSender<Msg>, UnboundedReceiver<Msg>) {
    mpsc::unbounded_channel()
}

/// What the launch states before the loop starts.
pub(crate) struct Surface {
    /// The model the surface opens on: the session's own facts.
    pub(crate) model: Model,
    /// The queue's two ends, and the front the session was composed over.
    pub(crate) msgs: UnboundedSender<Msg>,
    pub(crate) queue: UnboundedReceiver<Msg>,
    pub(crate) front: Arc<Forward>,
    /// The session the run was opened with, and the operations its commands
    /// state ([`super::ops`]).
    pub(crate) session: Arc<ChatSession>,
    pub(crate) ops: OpsContext,
    pub(crate) deps: Arc<OpsDeps>,
    /// What a direct source's commands need from the launch.
    pub(crate) direct: Arc<DirectDeps>,
    /// What opening a job needs from the launch (`/start`, `02` §8).
    pub(crate) jobs: Arc<JobDeps>,
    /// The configuration in force, which `/reload` and the post-setup apply
    /// swap. The same handle as `direct.runtime`'s.
    pub(crate) runtime: Arc<LiveRuntime>,
    /// The session's state, shared with the launch: a reload re-seeds it.
    pub(crate) state: Arc<Mutex<SessionState>>,
    /// The launch's own options: what `/reload` re-reads and what the guided
    /// setup's flow is built from.
    pub(crate) options: GlobalOptions,
    /// The facts every frame is painted with.
    pub(crate) facts: Facts,
    /// The session's metrics collector, when local metrics are on.
    pub(crate) metrics: Option<Arc<LocalMetrics>>,
    /// The instant the launch began: the first paint's sample is measured from
    /// it, as the surface this one replaces measured it.
    pub(crate) entered: Instant,
}

/// Runs the surface on the real terminal: its producers, its loop, and the
/// terminal the loop enters and leaves.
pub(crate) async fn run(surface: Surface) -> io::Result<()> {
    let Surface {
        model,
        msgs,
        queue,
        front,
        session,
        ops,
        deps,
        direct,
        jobs,
        runtime,
        state,
        options,
        facts,
        metrics,
        entered,
    } = surface;
    let port = Arc::new(SessionPort::new(PortParts {
        session,
        front: Arc::clone(&front),
        ops,
        direct,
        deps,
        options,
        runtime,
        state,
        jobs,
    }));
    let input = Arc::new(InputGate::new());
    let runtime = Runtime::new(model, queue, TtyScreen::new(), SystemClock, port)
        .with_facts(facts)
        .with_metrics(metrics, entered)
        .with_input(Arc::clone(&input));
    run_input::spawn_input(msgs.clone(), input);
    // A job's worker reports on queues of its own, so the panel is looked at on
    // a thread of its own: an idle surface paints no frame to read them on.
    run_input::spawn_jobs(Arc::clone(&front));
    run_input::spawn_ticker(runtime.animating_flag(), msgs, super::view::TICK);
    runtime.run().await
}

/// The first paint's sample: one metric, from the instant the launch began to
/// the frame the operator first saw.
pub(crate) fn record_first_paint(metrics: &LocalMetrics, entered: Instant) {
    metrics.record(&AgentMetric::Phase {
        phase: AgentMetricPhase::FirstPaint,
        elapsed_ns: u64::try_from(entered.elapsed().as_nanos()).unwrap_or(u64::MAX),
        outcome: AgentMetricOutcome::Succeeded,
    });
}
