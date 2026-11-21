//! A run port that records every call, and the ops a test builds over it.
//!
//! The point of the recording is the negative: a command that must stay outside
//! the run is proved so by a port that saw nothing at all.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use saya_agent::{AgentEventSink, ApprovalPolicy};

use super::ChatOps;
use crate::agent::tools::shell_policy::ModeCell;
use crate::chat::ChatOutcome;
use crate::chat::ops_context::OpsContext;
use crate::chat::port::{PortError, Prepared, RunPort};

/// A run port that answers every call and records it.
#[derive(Default)]
pub(super) struct RecordingPort {
    seen: Mutex<Vec<&'static str>>,
}

impl RecordingPort {
    /// Every call that reached the port, in order.
    pub(super) fn calls(&self) -> Vec<&'static str> {
        self.seen().clone()
    }

    fn note(&self, call: &'static str) {
        self.seen().push(call);
    }

    fn seen(&self) -> MutexGuard<'_, Vec<&'static str>> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl RunPort for RecordingPort {
    async fn prepare(&self) -> Result<Prepared, String> {
        self.note("prepare");
        Ok(Prepared::default())
    }

    async fn next_turn(
        &self,
        _sink: Arc<dyn AgentEventSink>,
        _prompt: String,
    ) -> Result<ChatOutcome, PortError> {
        self.note("next_turn");
        Ok(ChatOutcome::Answered)
    }

    async fn cancel_turn(&self) {
        self.note("cancel_turn");
    }

    async fn close(&self) {
        self.note("close");
    }
}

/// Ops over a recording port, with the run's cell and `bypass` in reach of the
/// test: the cell is what a mode switch moves, so a test reads it afterwards.
pub(super) struct Harness {
    pub(super) ops: ChatOps,
    pub(super) port: Arc<RecordingPort>,
    pub(super) cell: ModeCell,
}

/// A harness with the run's cell at `mode`, and `bypass` allowed or withheld.
pub(super) fn harness(mode: ApprovalPolicy, allow_bypass: bool) -> Harness {
    let port = Arc::new(RecordingPort::default());
    let cell = ModeCell::new(mode);
    let run: Arc<dyn RunPort> = port.clone();
    let ops = ChatOps::new(run, OpsContext::new(cell.clone(), allow_bypass));
    Harness { ops, port, cell }
}

/// One command, parsed the way a surface parses it.
pub(super) fn command(text: &str) -> crate::slash::SlashCommand {
    crate::slash::parse_slash_command(text)
        .unwrap_or_else(|error| panic!("`{text}` should parse: {error}"))
        .unwrap_or_else(|| panic!("`{text}` should be a slash command"))
}
