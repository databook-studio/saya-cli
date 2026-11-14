//! Answering and scrolling the pending tool-approval request.

use super::super::super::transcript::BlockKind;
use super::super::super::types::App;
use saya_agent::ApprovalChoice;

impl App {
    /// Answers the pending tool-approval request: the user's [`ApprovalChoice`]
    /// goes back to the turn's decider, which records it into the session policy
    /// (a session grant survives the turn; allow-once and deny leave nothing
    /// behind). The transcript names what the answer actually did.
    pub(crate) fn answer_approval(&mut self, choice: ApprovalChoice) {
        if let Some(pending) = self.request.pending_approval.take() {
            let verb = match &choice {
                ApprovalChoice::AllowOnce => format!("Approved tool: {}", pending.tool),
                ApprovalChoice::AllowSession { token } => {
                    format!("Allowed for session ({token}): {}", pending.tool)
                }
                ApprovalChoice::Deny => format!("Denied tool: {}", pending.tool),
            };
            let _ = pending.respond.send(choice);
            self.transcript.push(BlockKind::System, verb);
        }
    }

    /// Scrolls the pending approval's fact body by `delta` wrapped rows
    /// (negative scrolls up). The offset floors at the top here; the paint
    /// clamps the bottom against the live panel geometry, so no delta can
    /// scroll the facts out of reach. The offset dies with the approval —
    /// a fresh one starts at the top.
    pub(crate) fn scroll_approval(&mut self, delta: isize) {
        if let Some(pending) = &mut self.request.pending_approval {
            pending.scroll = pending.scroll.saturating_add_signed(delta);
        }
    }
}
