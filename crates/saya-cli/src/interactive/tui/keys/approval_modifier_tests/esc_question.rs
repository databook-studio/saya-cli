//! Esc on a live question stops the shared question itself, not merely the
//! approval modal, and drops the pending reply sender so no stale answer
//! can cross turns.

use super::*;

use crate::interactive::tui::agent::{Stream, StreamMsg};
use saya_agent::CancellationToken;
use tokio::sync::oneshot::error::TryRecvError;

/// Esc on a live request stops the shared question instead of merely denying
/// this modal; dropping the sender prevents a stale reply from crossing turns.
#[test]
fn esc_cancels_the_live_question_and_drops_the_pending_reply() {
    let (mut app, mut answer) = pending_approval(None);
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel::<StreamMsg>();
    let cancellation = CancellationToken::new();
    app.request.stream = Some(Stream {
        rx,
        cancel: cancellation.clone(),
        prompt: "question".into(),
        question_scope: None,
    });

    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);

    assert!(
        cancellation.is_cancelled(),
        "Esc must stop the actual question"
    );
    assert!(
        app.request.pending_approval.is_none(),
        "the modal must clear"
    );
    assert!(
        matches!(answer.try_recv(), Err(TryRecvError::Closed)),
        "a cleared modal must not retain an approval sender"
    );
}
