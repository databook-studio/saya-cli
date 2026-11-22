//! What the runtime does with the commands an update answered with.
//!
//! [`update`](super::update) decides and never performs; this is the other half.
//! A [`Cmd`] reaches the session's own API — send, cancel, answer, state a mode
//! — through one small trait, so the loop can be driven by a fake in a test and
//! by the real session in a run, and so nothing above the session invents a
//! second way to send a turn. The real port lives next door
//! ([`super::run_cmd_port`]); this file is the boundary both answer to.

use std::io;

use saya_agent::PermissionMode;

use super::direct::Direct;
use super::msg::Cmd;
use crate::agent::tools::consent::Answer;
use crate::interactive::tui::TerminalModes;
use crate::slash::SlashCommand;

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
    /// Cancel the job this id names, through today's job plumbing (`02` §8's
    /// `x`, once its confirm has been answered).
    fn cancel_job(&self, id: String);
    /// Perform one job verb (`02` §8): `/start` opens the run's one job through
    /// today's worker, and the rest steer the job it opened.
    fn job(&self, request: crate::slash::JobRequest);
    /// Perform one line the surface does not answer itself: a session
    /// operation, named by the chat layer's table ([`super::ops`]).
    fn op(&self, command: SlashCommand);
    /// Re-read the configuration and swap the runtime this surface runs on:
    /// `/reload`, and the apply a closed guided setup runs ([`super::reload`]).
    fn reload(&self);
    /// Hand the terminal to the guided setup and apply what it wrote: `/setup`.
    /// The terminal is the loop's, so the loop passes its mode pair here; the
    /// port owns the launch's options, the session and the words.
    fn guided_setup(&self, modes: &mut (dyn TerminalModes + Send)) -> io::Result<()>;
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
        Cmd::CancelJob(id) => port.cancel_job(id),
        Cmd::Job(request) => port.job(request),
        Cmd::Op(command) => port.op(command),
    }
}
