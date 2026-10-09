use super::super::super::transcript::BlockKind;
use super::super::super::types::App;
use super::super::approvals::{ApprovalKey, approval_answer, approval_key};
use super::cancel::request_stream_stop;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// The decision modals — tool approval, plan approval, startup trust — which
/// consume every key while open; `true` when the key was consumed.
pub(super) fn route(app: &mut App, code: KeyCode, mods: KeyModifiers) -> bool {
    // A tool-approval modal captures input until answered. One decision
    // function sorts scroll from answer: the keys that reach the withheld
    // facts can never be the keys that answer, and the modal's offered
    // token decides whether `s` is an answer at all.
    if let Some(pending) = app.request.pending_approval.as_ref() {
        if code == KeyCode::Esc && mods.is_empty() && app.request.stream.is_some() {
            if let Some(stream) = &app.request.stream {
                request_stream_stop(stream);
            }
            app.request.pending_approval = None;
            app.transcript.push(
                BlockKind::System,
                "Stop requested — waiting for the worker to confirm.",
            );
            return true;
        }
        match approval_key(code, mods, pending.grant.as_deref()) {
            ApprovalKey::Answer(choice) => app.answer_approval(choice),
            ApprovalKey::Scroll(delta) => app.scroll_approval(delta),
            ApprovalKey::Ignore => {}
        }
        return true;
    }
    // The run panel's plan-approval modal captures input the same way: the
    // merged M1-10 gate, answered by the UI, never stdin.
    if app
        .run_panel
        .as_ref()
        .is_some_and(|panel| panel.plan_approval.is_some())
    {
        if let Some(allow) = approval_answer(code, mods) {
            app.answer_plan_approval(allow);
        }
        return true;
    }
    // The startup trust modal captures input until answered: one decision —
    // trust this folder, name another directory, or continue unbound — with
    // the same words the plain REPL's line prompt carries. Answering binds
    // through `SessionRuntime::bind_trusted`, exactly like `--workspace`;
    // a bad directory refuses inline (the modal stays, with the error).
    if app.overlays.trust.is_some() {
        if let Some(dir) = app.answer_trust(code, mods) {
            let _ = dir;
        }
        return true;
    }
    false
}
