//! The saved-investigation picker's behaviour: the bounded load, the pure
//! filter, and the key actions. Opened by bare `/investigations`; Enter
//! shows the selected one through the same adapter `/investigation show
//! <id>` uses, `r` starts the same background replay `/investigation run
//! <id>` dispatches, Esc closes. The picker never writes to the store.

use super::super::dispatch::Dispatch;
use super::super::dispatch_investigation::run_investigation;
use super::super::replay::relative_time;
use super::super::transcript::BlockKind;
use super::super::types::{App, InvestigationEntry, InvestigationPicker};
use crate::cli::InvestigationCommand;
use crate::render::RenderFormat;
use saya_store::{InvestigationRepository, MAX_LIST_PAGE};

/// The most summaries the picker loads when opened: the collection cap,
/// read as at most ten pages of fifty.
const PICKER_MAX_ITEMS: usize = 500;
const PICKER_PAGES: usize = PICKER_MAX_ITEMS / MAX_LIST_PAGE;

/// Entries passing the query: a case-insensitive substring match over the id
/// and the display name. Pure: same inputs, same outputs.
pub(crate) fn filter_entries(
    entries: &[InvestigationEntry],
    query: &str,
) -> Vec<InvestigationEntry> {
    let needle = query.to_lowercase();
    entries
        .iter()
        .filter(|entry| {
            needle.is_empty()
                || entry.id.to_lowercase().contains(&needle)
                || entry.name.to_lowercase().contains(&needle)
        })
        .cloned()
        .collect()
}

impl App {
    /// Opens the investigation picker with the saved investigations, loaded
    /// once on a worker thread. A load already in flight or a picker already
    /// open is a no-op.
    pub(crate) fn open_investigation_picker(&mut self) {
        if self.overlays.investigations.is_some() || self.overlays.investigations_loading.is_some()
        {
            return;
        }
        let root = self.runtime.investigations_root.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(load_investigation_entries(&root));
        });
        self.overlays.investigations_loading = Some(receiver);
        self.transcript
            .push(BlockKind::System, "Loading saved investigations…");
    }

    /// Applies a background investigation-picker load once it completes. An
    /// empty store is said (with the save remedy), never rendered as a modal.
    pub(crate) fn poll_investigation_picker(&mut self) {
        let result = self
            .overlays
            .investigations_loading
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                    "investigation picker worker stopped unexpectedly".into(),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
        let Some(result) = result else { return };
        self.overlays.investigations_loading = None;
        match result {
            Ok((entries, _)) if entries.is_empty() => self.transcript.push(
                BlockKind::System,
                "No saved investigations — save one with /investigation save <name>",
            ),
            Ok((entries, capped)) => {
                self.overlays.investigations = Some(InvestigationPicker {
                    entries,
                    selected: 0,
                    capped,
                    query: String::new(),
                });
            }
            Err(error) => self.transcript.push(BlockKind::Error, error),
        }
    }

    /// Entries passing the picker's current filter, in list order.
    pub(crate) fn investigations_visible(
        &self,
        picker: &InvestigationPicker,
    ) -> Vec<InvestigationEntry> {
        filter_entries(&picker.entries, &picker.query)
    }

    /// Extends the picker's filter; resets the selection to the first match.
    pub(crate) fn investigations_char(&mut self, c: char) {
        if let Some(picker) = &mut self.overlays.investigations {
            picker.query.push(c);
            picker.selected = 0;
        }
    }

    pub(crate) fn investigations_backspace(&mut self) {
        if let Some(picker) = &mut self.overlays.investigations {
            picker.query.pop();
            picker.selected = 0;
        }
    }

    /// Moves the picker selection by `delta`, clamped to the filtered list.
    pub(crate) fn investigations_move(&mut self, delta: isize) {
        let len = match self.overlays.investigations.as_ref() {
            Some(picker) => filter_entries(&picker.entries, &picker.query).len(),
            None => return,
        };
        if let Some(picker) = &mut self.overlays.investigations {
            if len == 0 {
                return;
            }
            let next = (picker.selected as isize + delta).clamp(0, len as isize - 1);
            picker.selected = next as usize;
        }
    }

    /// Confirms the picker selection: the chosen investigation is shown
    /// through the same adapter `/investigation show <id>` uses, so the
    /// transcript block is exactly the one that command produces.
    pub(crate) fn investigations_confirm(&mut self) {
        if let Some(picker) = self.overlays.investigations.take()
            && let Some(id) = self.chosen_id(&picker)
        {
            self.show_investigation(&id);
        }
    }

    /// Runs the selected investigation: the same background replay
    /// `/investigation run <id>` dispatches — a [`Dispatch::ReplayTask`]
    /// admitted through `start_replay`, worker-permit cap and busy refusal
    /// included. A replay still needing `--connection` fails inside the
    /// worker with the same refusal the command shows.
    pub(crate) fn investigations_run(&mut self) {
        if let Some(picker) = self.overlays.investigations.take()
            && let Some(id) = self.chosen_id(&picker)
        {
            let command = InvestigationCommand::Run {
                id,
                connection: None,
                revalidate: false,
                report: None,
                rows: None,
                overwrite: false,
            };
            if let Some(Dispatch::ReplayTask(task)) = self.dispatch_investigation_command(command) {
                self.start_replay(task);
            }
        }
    }

    fn chosen_id(&self, picker: &InvestigationPicker) -> Option<String> {
        self.investigations_visible(picker)
            .get(picker.selected)
            .map(|entry| entry.id.clone())
    }

    /// One investigation command through the shared adapter with the
    /// session's resolved output format — the same fallback the launch path
    /// uses when no `--format` override was given.
    fn dispatch_investigation_command(
        &mut self,
        command: InvestigationCommand,
    ) -> Option<Dispatch> {
        run_investigation(
            &mut self.transcript,
            &self.runtime,
            &self.state_db,
            RenderFormat::from(self.runtime.resolved.output_format),
            &command,
            &self.last_query,
        )
    }

    fn show_investigation(&mut self, id: &str) {
        let _ =
            self.dispatch_investigation_command(InvestigationCommand::Show { id: id.to_string() });
    }
}

/// Loads at most [`PICKER_MAX_ITEMS`] summaries from the repository root: at
/// most ten pages of fifty, read once, no writes. Returns the entries plus
/// whether more exist beyond the bound.
fn load_investigation_entries(
    root: &std::path::Path,
) -> Result<(Vec<InvestigationEntry>, bool), String> {
    let repository = InvestigationRepository::new(root.to_path_buf());
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut entries: Vec<InvestigationEntry> = Vec::new();
    let mut capped = false;
    let mut offset = 0;
    for _ in 0..PICKER_PAGES {
        let page = repository
            .list(offset, MAX_LIST_PAGE)
            .map_err(|error| error.to_string())?;
        entries.extend(page.summaries.iter().map(|summary| {
            let when = relative_time(now_ms.saturating_sub(summary.updated_unix_ms.max(0) as u128));
            InvestigationEntry {
                id: summary.id.as_str().to_owned(),
                name: summary.name.clone(),
                label: format!(
                    "{when:<10}  {}  ·  {}  ·  {}",
                    summary.name,
                    summary.dialect.as_str(),
                    summary.connection
                ),
            }
        }));
        offset += MAX_LIST_PAGE;
        // More exist than the picker can show only when candidates remain
        // that no page read: past the picker's bound, or past the scan's
        // own bound (the collection cap + 1).
        capped = page.total_seen > offset;
        if entries.len() >= PICKER_MAX_ITEMS || offset >= page.total_seen {
            break;
        }
    }
    entries.truncate(PICKER_MAX_ITEMS);
    Ok((entries, capped))
}

#[cfg(test)]
#[path = "investigation_picker_tests.rs"]
mod tests;
