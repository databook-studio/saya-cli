//! Scrolling reaches the withheld facts (E2, audit F05's second half): the
//! whole fact body is reachable at any supported terminal size, and no key
//! used to reach it can ever answer the modal. Real frames through the real
//! `ui::draw`, real key events through the real `handle_key`.

use crate::interactive::tui::keys::handle_key;
use crate::interactive::tui::types::{App, PendingApproval};
use crate::interactive::tui::ui_snapshot_tests::{empty_app, fixed_status, render_buffer};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use saya_agent::ApprovalChoice;
use tokio::sync::oneshot::error::TryRecvError;

/// The audit's repro, longer still: a body whose wrapped rows far outrun the
/// capped panel, so the tail of the facts is withheld until scrolled to.
fn long_detail() -> String {
    (1..=30)
        .map(|i| format!("fact line {i:02} of the consequence text"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A second, differently-worded body, so `a_fresh_approval_starts_at_the_top`
/// can tell the two approvals' texts apart in the buffer.
fn other_long_detail() -> String {
    (1..=30)
        .map(|i| format!("second call fact {i:02} of its own consequence"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// An idle app with a pending approval carrying `detail` and `grant`, plus
/// the answer channel a test watches for anything reaching the decider.
fn approval_app(
    detail: Option<String>,
    grant: Option<&str>,
) -> (App, tokio::sync::oneshot::Receiver<ApprovalChoice>) {
    let mut app = empty_app();
    let (respond, answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(PendingApproval {
        tool: "http_fetch".into(),
        detail,
        grant: grant.map(str::to_string),
        scroll: 0,
        respond,
    });
    (app, answer)
}

#[cfg(test)]
#[path = "approval_scroll_tests/answer_keys.rs"]
mod answer_keys;
#[cfg(test)]
#[path = "approval_scroll_tests/scrolling.rs"]
mod scrolling;
