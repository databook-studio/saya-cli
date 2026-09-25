//! The terminal side of the plan-approval channel: when a `saya run` can
//! interact (a terminal, not `--non-interactive`), the bound plan is shown
//! once and answered once. The channel shape is the `tui/agent.rs` pattern —
//! the request travels with a oneshot reply — so the TUI's modal (a later
//! slice) swaps this driver without touching the seam. stdin carries the
//! answer; stdout stays the run's output surface.

use super::approval::{PlanApproval, PlanApprovalRequest};
use tokio::sync::mpsc::UnboundedReceiver;

/// The approval surface for a run that can ask: the channel with this
/// terminal driver answering. The surface outlives the first ask — a
/// mid-run capability re-ask (DESIGN §5.3) rides the same channel rather
/// than inheriting anything.
pub(super) fn terminal() -> PlanApproval {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(answer(receiver));
    PlanApproval::ViaChannel(sender)
}

/// Answers every ask the run sends, in order, until the channel closes.
async fn answer(mut receiver: UnboundedReceiver<PlanApprovalRequest>) {
    while let Some(request) = receiver.recv().await {
        let view_text = request.view_text;
        // The terminal read is a blocking interaction, so it runs on the
        // blocking pool: the runtime stays responsive and Ctrl-C still
        // cancels the run while the prompt is open.
        let answer = tokio::task::spawn_blocking(move || read_y_n(&view_text))
            .await
            .unwrap_or(false);
        let _ = request.respond.send(answer);
    }
}

/// Renders the view and reads one answer. A non-terminal stdin cannot be
/// asked, and anything but an explicit yes is a refusal.
fn read_y_n(view_text: &str) -> bool {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprintln!("{view_text}");
    eprint!("Approve this plan, its scopes, and its budgets? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    match std::io::stdin().lock().read_line(&mut answer) {
        Ok(_) => decides(&answer),
        Err(_) => false,
    }
}

/// Whether one typed line approves: an explicit yes and nothing else.
fn decides(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::decides;

    #[test]
    fn only_an_explicit_yes_approves() {
        assert!(decides("y"));
        assert!(decides("yes\n"));
        assert!(decides("  Y  "));
        assert!(!decides("n"));
        assert!(!decides(""));
        assert!(!decides("\n"));
        assert!(!decides("approve"));
    }
}
