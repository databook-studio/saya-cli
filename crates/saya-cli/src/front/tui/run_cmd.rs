//! What the runtime does with the commands an update answered with.
//!
//! [`update`](super::update) decides and never performs; this is the other half.
//! A [`Cmd`] reaches the session's own API — send, cancel, answer, state a mode
//! — through one small trait, so the loop can be driven by a fake in a test and
//! by the real session in a run, and so nothing above the session invents a
//! second way to send a turn.

use std::sync::Arc;

use saya_agent::PermissionMode;

use super::direct::Direct;
use super::direct_run::{DirectDeps, perform as perform_direct};
use super::direct_trust;
use super::msg::Cmd;
use super::run_front::Forward;
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatSession, OpsContext};
use crate::interactive::tui::{TrustResolution, copy_to_native_clipboard, osc52_copy};

/// What a command is performed on: the session that runs the turns, and the
/// front that holds the operator's unanswered calls.
pub(crate) trait CmdPort: Send + Sync {
    /// Run one message as a turn.
    fn send(&self, message: String);
    /// Ask the turn in flight to stop.
    fn cancel(&self);
    /// Answer the call put under `id`.
    fn answer(&self, id: u64, answer: Answer);
    /// State a mode on the run's cell.
    fn switch_mode(&self, mode: PermissionMode);
    /// Put this text on the clipboard (`02` §6's `c`).
    fn copy(&self, text: String);
    /// Run one command the surface answers itself (`03` §4.1): the query, the
    /// export, the chart, the investigation.
    fn direct(&self, command: Direct);
    /// Answer the workspace-trust question by binding the launch folder
    /// (`02` §9.4): today's `bind_trusted`, exactly like `--workspace`.
    fn trust(&self);
}

/// Performs one command on the port that can.
pub(crate) fn perform(port: &dyn CmdPort, cmd: Cmd) {
    match cmd {
        Cmd::Send(message) => port.send(message),
        Cmd::CancelTurn => port.cancel(),
        Cmd::Answer(id, answer) => port.answer(id, answer),
        Cmd::SwitchMode(mode) => port.switch_mode(mode),
        Cmd::SetMode(mode) => port.switch_mode(mode),
        Cmd::Copy(text) => port.copy(text),
        Cmd::Direct(command) => port.direct(command),
        Cmd::Trust => port.trust(),
    }
}

/// The real port: the session's API, the front's pending asks, the chat
/// layer's one author of a mode switch, and the launch's direct dependencies.
pub(crate) struct SessionPort {
    session: Arc<ChatSession>,
    front: Arc<Forward>,
    ops: OpsContext,
    direct: Arc<DirectDeps>,
}

impl SessionPort {
    /// A port over one session, the front the session talks to, the run's
    /// operations context, and what its direct commands need from the launch.
    pub(crate) fn new(
        session: Arc<ChatSession>,
        front: Arc<Forward>,
        ops: OpsContext,
        direct: Arc<DirectDeps>,
    ) -> Self {
        Self {
            session,
            front,
            ops,
            direct,
        }
    }
}

impl CmdPort for SessionPort {
    fn send(&self, message: String) {
        // The turn runs on a task of its own: its events come back through the
        // front, so the loop keeps folding them and reading keys while the
        // session works. A turn that takes the loop's thread would freeze the
        // surface it is drawing.
        let session = Arc::clone(&self.session);
        tokio::spawn(async move {
            let _ = session.send(message).await;
        });
    }

    fn cancel(&self) {
        let session = Arc::clone(&self.session);
        tokio::spawn(async move {
            session.cancel().await;
        });
    }

    fn answer(&self, id: u64, answer: Answer) {
        self.front.answer(id, answer);
    }

    fn switch_mode(&self, mode: PermissionMode) {
        // A refusal is the operator's business, and the front is how this port
        // says anything: it lands as the surface's own words, never dropped.
        if let Err(refusal) = self.ops.state_mode(mode) {
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
}
