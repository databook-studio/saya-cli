use super::transcript::BlockKind;
use super::types::{App, SessionSave};
use crate::interactive::session::lifecycle::capacity::{limit_reached_text, notice_after_save};
use crate::interactive::session::lifecycle::resume::block_on;
use crate::interactive::session::state::SessionState;
use saya_store::{FsSessionStore, SessionStore, StoreError};

/// The user-facing sentence a failed background save renders: a refusal at
/// the 4 MiB record cap gets its own remedy wording; every other store error
/// keeps the lost-changes warning with the raw error text.
fn save_failure_text(error: StoreError, session_id: &str) -> String {
    match error {
        StoreError::LimitExceeded => limit_reached_text(session_id),
        other => format!("Could not save this session; your latest changes may be lost: {other}"),
    }
}

fn start_session_save(app: &mut App, store: &FsSessionStore, session: saya_store::RedactedSession) {
    let store = store.clone();
    let session_id = session.id.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result =
            block_on(store.save(session)).map_err(|error| save_failure_text(error, &session_id));
        let _ = sender.send(result);
    });
    app.session_save = Some(SessionSave { result: receiver });
}

pub(super) fn queue_session_save(app: &mut App, store: &FsSessionStore, state: &SessionState) {
    let session = state.redacted();
    if app.session_save.is_some() {
        app.pending_session_save = Some(session);
    } else {
        start_session_save(app, store, session);
    }
}

pub(super) fn poll_session_save(app: &mut App, store: &FsSessionStore, state: &mut SessionState) {
    let result = app
        .session_save
        .as_ref()
        .and_then(|save| match save.result.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                "Could not save this session; your latest changes may be lost: session save \
                 worker stopped unexpectedly"
                    .to_string(),
            )),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        });
    let Some(result) = result else { return };
    app.session_save = None;
    match result {
        Ok(()) => {
            if let Some(text) = notice_after_save(store, state) {
                app.transcript.push(BlockKind::System, text);
            }
        }
        Err(text) => app.transcript.push(BlockKind::Error, text),
    }
    if let Some(session) = app.pending_session_save.take() {
        start_session_save(app, store, session);
    }
}

#[cfg(test)]
#[path = "session_save_tests.rs"]
mod session_save_tests;
