//! The real port: the session's API, the front's pending asks, and the launch's
//! own configuration and state — what each command is actually performed on.
//!
//! It is a second file because it is a second concern: [`super::run_cmd`] states
//! what a performer must offer, and this is the only implementation of it a run
//! uses. Everything here is today's code called, never re-written: a turn runs
//! through the chat session's own `send`, a direct command through
//! [`super::direct_run`], and a reload through the crate's reload core.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use saya_agent::PermissionMode;

use super::direct::Direct;
use super::direct_run::{DirectDeps, perform as perform_direct};
use super::direct_trust;
use super::jobs_open::JobDeps;
use super::line_surface::SurfaceLine;
use super::live::LiveRuntime;
use super::msg::Worker;
use super::ops::{self, Asked, OpsDeps};
use super::reload;
use super::run_cmd::CmdPort;
use super::run_front::Forward;
use super::setup;
use crate::agent::tools::consent::Answer;
use crate::cli::GlobalOptions;
use crate::config::reload::ReloadInputs;
use crate::interactive::SessionState;
use crate::interactive::tui::{
    TerminalModes, TrustResolution, copy_to_native_clipboard, osc52_copy,
};
use crate::slash::SlashCommand;

/// Everything the real port is built from: the session's API, the front's
/// pending asks, the chat layer's one author of a mode switch, what the launch's
/// direct commands need, the launch's own options, the configuration and
/// session state a reload moves, and what opening a job needs.
pub(crate) struct PortParts {
    /// The line the surface drives: the run it is on now, its transcript, its
    /// compaction and its rollover — the same session the line loop holds.
    pub(crate) line: Arc<SurfaceLine>,
    pub(crate) front: Arc<Forward>,
    pub(crate) direct: Arc<DirectDeps>,
    pub(crate) deps: Arc<OpsDeps>,
    pub(crate) options: GlobalOptions,
    pub(crate) runtime: Arc<LiveRuntime>,
    pub(crate) state: Arc<Mutex<SessionState>>,
    /// What opening a job needs from the launch (`/start`, `02` §8).
    pub(crate) jobs: Arc<JobDeps>,
    /// Whether `bypass` is reachable at all in this launch: the answer the mode
    /// switch is stated with.
    pub(crate) allow_bypass: bool,
}

/// The real port.
pub(crate) struct SessionPort {
    pub(super) line: Arc<SurfaceLine>,
    pub(super) front: Arc<Forward>,
    pub(super) allow_bypass: bool,
    direct: Arc<DirectDeps>,
    pub(super) deps: Arc<OpsDeps>,
    /// The launch's own options: what `/reload` re-reads and what the guided
    /// setup's flow is built from, exactly as the old surface built it.
    options: GlobalOptions,
    /// The configuration in force, shared with the direct commands.
    runtime: Arc<LiveRuntime>,
    /// The session's state, shared with the launch: a reload re-seeds it, and
    /// the record the launch saves afterwards carries the re-seed.
    state: Arc<Mutex<SessionState>>,
    /// What opening a job needs from the launch (`/start`, `02` §8).
    pub(super) jobs: Arc<JobDeps>,
    /// The one-release notes the job spellings have already said in this
    /// session (`chat::ops_job`).
    pub(super) notes: Mutex<crate::chat::JobNotes>,
}

impl SessionPort {
    /// A port over one session and everything a command of the surface's needs.
    pub(crate) fn new(parts: PortParts) -> Self {
        let PortParts {
            line,
            front,
            direct,
            deps,
            options,
            runtime,
            state,
            jobs,
            allow_bypass,
        } = parts;
        Self {
            line,
            front,
            allow_bypass,
            direct,
            deps,
            options,
            runtime,
            state,
            jobs,
            notes: Mutex::new(crate::chat::JobNotes::default()),
        }
    }

    /// The one reload: the crate's re-read over the launch's options, the swap,
    /// and the plan's own words as the surface's answer.
    fn reload_asked(&self) -> Asked {
        let inputs = ReloadInputs::from_launch(&self.options);
        reload::apply(&inputs, &self.runtime, &mut self.state())
    }

    /// Says what an operation answered: a notice, the refusal it is, or the
    /// loop leaving — the surface's one way of speaking.
    fn say(&self, asked: Asked) {
        match asked {
            Asked::Notice(words) => self.front.deliver(Worker {
                direct: Some(super::direct::DirectOutcome::Notice(words)),
                ..Worker::default()
            }),
            Asked::Error(words) => self.front.deliver(Worker {
                direct: Some(super::direct::DirectOutcome::Error(words)),
                ..Worker::default()
            }),
            Asked::Quit => self.front.quit(),
            // The reload's own answer is its report, so saying it is doing it —
            // and `reload` answers with words only, never with this arm.
            Asked::Reload => self.reload(),
            // `/setup` is the loop's: it intercepts the command and hands the
            // terminal over before this table is reached. The arm keeps the
            // match total, and says nothing rather than saying something twice.
            Asked::Setup => {}
            // `/continue` is the one session op that states a turn: the chat
            // session sends the paused turn's message through its own path.
            Asked::Continue => self.line.continue_turn(),
            Asked::Job(request) => self.perform_job(request),
            // A rollover: the conversation's own two are the chat layer's
            // compaction and clear, which the session performs; the change
            // waits for a turn in flight, exactly as the line front's rule says.
            Asked::Roll { request, said } => self.roll(request, said),
            // One shared operation the surface reads or writes: it takes as long
            // as it takes, so it runs off the loop's thread and comes back as an
            // entry in the transcript.
            Asked::Work(work) => self.work(work),
        }
    }

    /// The session's state, with a poisoned lock read as the value it last
    /// held: a panicking reader must not take the session's own state with it.
    pub(super) fn state(&self) -> MutexGuard<'_, SessionState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl CmdPort for SessionPort {
    fn send(&self, message: String) {
        // The turn runs on a task of its own: its events come back through the
        // front, so the loop keeps folding them and reading keys while the
        // session works. A turn that takes the loop's thread would freeze the
        // surface it is drawing. It runs as a turn of the line the surface
        // holds, so the boundary that follows it — the automatic compaction
        // included — is the line front's own.
        self.line.send(message);
    }

    fn replace_last(&self, message: String) {
        // The line's own operation: the rewind runs before the turn, on the
        // line's thread, and its words are said where a notice lands.
        self.line.replace_last(message);
    }

    fn cancel(&self) {
        self.line.cancel();
    }

    fn answer(&self, id: u64, answer: Answer) {
        self.front.answer(id, answer);
    }

    fn switch_mode(&self, mode: PermissionMode) {
        // A refusal is the operator's business, and the front is how this port
        // says anything: it lands as the surface's own words, never dropped. The
        // cell is the run's own, read through the line, so a rollover's cell is
        // the one the next switch moves.
        let Some(ops) = self.line.context(self.allow_bypass) else {
            return;
        };
        if let Err(refusal) = ops.state_mode(mode) {
            self.front.say(refusal);
        }
    }

    fn copy(&self, text: String) {
        // Copying shells out to a clipboard tool and waits on it, so it runs off
        // the loop's thread the way a turn does: the surface keeps reading keys
        // while the clipboard works. The attempt is the crate's one: the OS
        // tool first, then the terminal's own OSC 52, which is what reaches a
        // local clipboard over SSH (today's rule, `interactive::tui`).
        tokio::task::spawn_blocking(move || {
            if copy_to_native_clipboard(&text) {
                return;
            }
            let _ = osc52_copy(&mut std::io::stdout(), &text);
        });
    }

    fn direct(&self, command: Direct) {
        // A direct command is a query or a file write: it takes as long as it
        // takes, so it runs on a blocking task and hands its answer back through
        // the front — never on the loop's thread, which is painting the surface.
        let deps = Arc::clone(&self.direct);
        let front = Arc::clone(&self.front);
        tokio::task::spawn_blocking(move || {
            front.deliver(perform_direct(&deps, command));
        });
    }

    fn trust(&self) {
        // A `y` binds the launch folder through today's store — the live
        // session runtime's `bind_trusted` — which recomposes the universe and
        // canonicalises a path, so it runs off the loop's thread too.
        let store = Arc::clone(&self.direct.trust);
        let front = Arc::clone(&self.front);
        tokio::task::spawn_blocking(move || {
            front.deliver(direct_trust::answer(
                store.as_ref(),
                TrustResolution::TrustCwd,
            ));
        });
    }

    fn cancel_job(&self, id: String) {
        // The panel's own cancellation, exactly as the old TUI's `App` does it:
        // a job of this run is one of the front's workers, and cancelling it is a
        // token or a control message, never a second job runner (`02` §8).
        self.front.cancel_job(&id);
    }

    fn job(&self, request: crate::slash::JobRequest) {
        // The verb and its one-release note are the job surface's own code
        // (`super::jobs_open`), stated beside the worker it opens.
        self.perform_job(request);
    }

    fn op(&self, command: SlashCommand) {
        // The chat layer's table answers, and the surface says what it said: a
        // refusal is the operator's business, `/exit` asks the loop to leave,
        // and `/reload` is performed here, with the launch's own options. A
        // stateless statement — no turn is entered either way.
        //
        // The table states its operations over the run the session is on NOW: a
        // rollover replaced the run, so its port and its mode cell are re-stated
        // here before the command that reads them.
        if let Some(port) = self.line.run_port()
            && let Some(context) = self.line.context(self.allow_bypass)
        {
            self.deps.restate(crate::chat::ChatOps::new(port, context));
        }
        self.say(ops::perform(self.deps.as_ref(), command));
    }

    fn reload(&self) {
        // The old surface's queued drain ran the same reload between turns; this
        // surface performs it in place, and only a candidate that built swaps
        // the configuration in force.
        let asked = self.reload_asked();
        self.say(asked);
    }

    fn guided_setup(&self, modes: &mut (dyn TerminalModes + Send)) -> io::Result<()> {
        // The old surface's busy gate: the wizard owns the screen, and a turn
        // streaming behind it would be painted under the wizard. A run in
        // flight is the operator's to cancel, and this says so.
        // A turn in flight — or a command that moves the run — holds the line.
        if self.line.busy() {
            self.front.say(setup::WHILE_BUSY);
            return Ok(());
        }
        let outcome = setup::run(modes, &self.options);
        // A completed setup — exit 0, including the wizard's own cancel — is
        // applied by the same reload `/reload` runs, without a restart.
        let asked = setup::landed(outcome, || self.reload_asked())?;
        self.say(asked);
        Ok(())
    }
}
