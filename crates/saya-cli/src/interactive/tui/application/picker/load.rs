//! Opening the session picker and loading its entries in the background.

use super::super::super::replay::relative_time;
use super::super::super::transcript::BlockKind;
use super::super::super::types::{App, Picker, PickerEntry};
use saya_store::{
    FsSessionStore, MAX_SESSION_HISTORY_PAGE_SIZE, SessionHistoryQuery, SessionStore,
};

impl App {
    /// Opens the session picker with the most recent saved sessions, enriched
    /// with each session's profile, model, turn count, and relative age.
    pub(crate) fn open_session_picker(&mut self, store: &FsSessionStore) {
        if self.overlays.picker.is_some() || self.overlays.picker_loading.is_some() {
            return;
        }
        let store = store.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(load_picker_entries(&store));
        });
        self.overlays.picker_loading = Some(receiver);
        self.transcript
            .push(BlockKind::System, "Loading saved sessions…");
    }

    /// Applies a background session-picker load once it completes.
    pub(crate) fn poll_session_picker(&mut self) {
        let result =
            self.overlays
                .picker_loading
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        Some(Err("session picker worker stopped unexpectedly".into()))
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                });
        let Some(result) = result else { return };
        self.overlays.picker_loading = None;
        match result {
            Ok((entries, _)) if entries.is_empty() => self
                .transcript
                .push(BlockKind::System, "No saved sessions to resume."),
            Ok((entries, has_more)) => {
                self.overlays.picker = Some(Picker {
                    entries,
                    selected: 0,
                    has_more,
                    query: String::new(),
                });
            }
            Err(error) => self.transcript.push(BlockKind::Error, error),
        }
    }
}

fn load_picker_entries(store: &FsSessionStore) -> Result<(Vec<PickerEntry>, bool), String> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let page = crate::interactive::session::lifecycle::resume::block_on(
        store.history(
            SessionHistoryQuery::first_page(MAX_SESSION_HISTORY_PAGE_SIZE)
                .expect("history page bound is valid"),
        ),
    )
    .map_err(|error| error.to_string())?;
    let has_more = page.has_more();
    let entries = page
        .entries
        .into_iter()
        .map(|entry| {
            let when = relative_time(now_ms.saturating_sub(entry.modified_unix_ms));
            let (profile, model, turns) =
                match crate::interactive::session::lifecycle::resume::block_on(
                    store.load(&entry.id),
                ) {
                    Ok(Some(session)) => (
                        session.profile.unwrap_or_else(|| "(no profile)".into()),
                        format!("{}/{}", session.provider, session.model),
                        session.turns.len(),
                    ),
                    _ => ("(no profile)".into(), "?".into(), 0),
                };
            PickerEntry {
                label: format!("{when:<10}  {profile:<16}  {model:<24}  {turns} turn(s)"),
                id: entry.id,
            }
        })
        .collect::<Vec<_>>();
    Ok((entries, has_more))
}
