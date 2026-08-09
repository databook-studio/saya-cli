//! Session picker, approval, schema refs, and transcript scrolling.

use super::super::atref;
use super::super::replay::{history_blocks, relative_time};
use super::super::transcript::BlockKind;
use super::super::types::{App, Picker, PickerEntry};
use crate::interactive::session_state::SessionState;
use saya_store::{FsSessionStore, SchemaStore, SessionStore};

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
            Ok(entries) if entries.is_empty() => self
                .transcript
                .push(BlockKind::System, "No saved sessions to resume."),
            Ok(entries) => {
                self.overlays.picker = Some(Picker {
                    entries,
                    selected: 0,
                });
            }
            Err(error) => self.transcript.push(BlockKind::Error, error),
        }
    }
}

fn load_picker_entries(store: &FsSessionStore) -> Result<Vec<PickerEntry>, String> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let entries = crate::interactive::session_resume::block_on(store.history())
        .map_err(|error| error.to_string())?
        .into_iter()
        .take(20)
        .map(|entry| {
            let when = relative_time(now_ms.saturating_sub(entry.modified_unix_ms));
            let (profile, model, turns) =
                match crate::interactive::session_resume::block_on(store.load(&entry.id)) {
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
    Ok(entries)
}

impl App {
    /// Moves the picker selection by `delta`, clamped.
    pub(crate) fn picker_move(&mut self, delta: isize) {
        if let Some(picker) = &mut self.overlays.picker {
            let len = picker.entries.len();
            if len == 0 {
                return;
            }
            let next = (picker.selected as isize + delta).clamp(0, len as isize - 1);
            picker.selected = next as usize;
        }
    }

    /// Confirms the picker selection, requesting a resume in the run loop.
    pub(crate) fn picker_confirm(&mut self) {
        if let Some(picker) = self.overlays.picker.take()
            && let Some(entry) = picker.entries.into_iter().nth(picker.selected)
        {
            self.overlays.pending_resume = Some(entry.id);
        }
    }

    /// Answers the pending tool-approval request and records the decision.
    pub(crate) fn answer_approval(&mut self, allow: bool) {
        if let Some(pending) = self.request.pending_approval.take() {
            let _ = pending.respond.send(allow);
            let verb = if allow { "Approved" } else { "Denied" };
            self.transcript
                .push(BlockKind::System, format!("{verb} tool: {}", pending.tool));
        }
    }

    /// Reloads `@`-reference names from the cached schema of the active and
    /// included profiles (best-effort; empty when nothing is cached).
    pub(crate) fn reload_at_refs(&mut self, state: &SessionState) {
        let mut names: Vec<&str> = Vec::new();
        if let Some(profile) = state.profile.as_deref() {
            names.push(profile);
        }
        names.extend(state.included_profiles.iter().map(String::as_str));
        let mut refs = Vec::new();
        for name in names {
            if let Ok(profile) = self.runtime.named_profile(name) {
                let identity = crate::profile_identity::profile_identity(
                    name,
                    profile,
                    &self.runtime.cache_scope,
                );
                if let Ok(Some(cached)) = crate::interactive::session_resume::block_on(
                    self.state_db.get_schema(&identity),
                ) {
                    refs.extend(atref::schema_refs(&cached.schema));
                }
            }
        }
        refs.sort();
        refs.dedup();
        self.at_refs = refs;
    }

    /// Replays a resumed session's saved turns into the transcript so the user
    /// sees the prior conversation instead of an empty panel. Does nothing for a
    /// session with no completed turns, leaving the welcome message in place.
    pub(crate) fn show_history(&mut self, state: &SessionState) {
        if state.turns.is_empty() {
            return;
        }
        self.transcript.clear();
        for (kind, text) in history_blocks(state) {
            self.transcript.push(kind, text);
        }
        self.transcript.scroll_to_bottom();
    }

    /// Scrolls the transcript by `delta` pages (negative = up), using the last
    /// rendered viewport height.
    pub(crate) fn scroll_pages(&mut self, up: bool) {
        let (_, height) = self.viewport.get();
        self.scroll_lines(up, (height as usize).saturating_sub(1).max(1));
    }

    /// Scrolls the transcript by `n` lines (for the mouse wheel).
    pub(crate) fn scroll_lines(&mut self, up: bool, n: usize) {
        let (width, height) = self.viewport.get();
        if up {
            self.transcript
                .scroll_up(n, width as usize, (height as usize).max(1));
        } else {
            self.transcript.scroll_down(n);
        }
    }
}
