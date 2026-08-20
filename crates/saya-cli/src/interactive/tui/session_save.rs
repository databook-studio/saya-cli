use super::super::session_resume::block_on;
use super::super::session_state::SessionState;
use super::transcript::BlockKind;
use super::types::{App, SessionSave};
use saya_store::{FsSessionStore, SessionStore};

fn start_session_save(app: &mut App, store: &FsSessionStore, session: saya_store::RedactedSession) {
    let store = store.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = block_on(store.save(session)).map_err(|error| error.to_string());
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

pub(super) fn poll_session_save(app: &mut App, store: &FsSessionStore) {
    let result = app
        .session_save
        .as_ref()
        .and_then(|save| match save.result.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("session save worker stopped unexpectedly".into()))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        });
    let Some(result) = result else { return };
    app.session_save = None;
    if let Err(error) = result {
        app.transcript.push(
            BlockKind::Error,
            format!("Could not save this session; your latest changes may be lost: {error}"),
        );
    }
    if let Some(session) = app.pending_session_save.take() {
        start_session_save(app, store, session);
    }
}
