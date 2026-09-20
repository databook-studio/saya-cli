//! UI overlays and modal interaction state.

use super::super::complete::Candidate;
use std::sync::mpsc::Receiver;

/// Live slash-command popup state.
pub(crate) struct Menu {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) selected: usize,
}

/// A selectable list of saved sessions to resume, filterable as you type.
pub(crate) struct Picker {
    pub(crate) entries: Vec<PickerEntry>,
    pub(crate) selected: usize,
    pub(crate) has_more: bool,
    /// Case-insensitive substring filter over id + label.
    pub(crate) query: String,
}

/// One row in the session picker.
#[derive(Clone)]
pub(crate) struct PickerEntry {
    pub(crate) id: String,
    pub(crate) label: String,
}

type PickerLoad = Result<(Vec<PickerEntry>, bool), String>;

/// UI overlays and modal interaction state.
/// A Ctrl+R (input history) or Ctrl+F (transcript) search overlay.
pub(crate) struct SearchOverlay {
    pub(crate) kind: SearchKind,
    pub(crate) query: String,
    /// Selected index into the filtered candidate list (history mode).
    pub(crate) selected: usize,
    /// The wrapped-line index the last Enter jumped to (transcript mode), so
    /// the next Enter walks to the *following* match instead of re-landing on
    /// the same one. `None` until the first jump, and reset whenever the query
    /// changes (so an edit restarts the search from the viewport top).
    pub(crate) last_match: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchKind {
    History,
    Transcript,
}

#[derive(Default)]
pub(crate) struct OverlayState {
    pub(crate) menu: Option<Menu>,
    pub(crate) picker_loading: Option<Receiver<PickerLoad>>,
    pub(crate) picker: Option<Picker>,
    pub(crate) pending_resume: Option<String>,
    pub(crate) show_help: bool,
    pub(crate) selection_mode: bool,
    pub(crate) search: Option<SearchOverlay>,
    /// A pending startup workspace-trust question: the TUI's rendering of
    /// the one trust decision — trust this folder, name another directory,
    /// or continue unbound — opened once after the splash paints. `None`
    /// everywhere else: the question never re-opens, and the plain REPL's
    /// line prompt is a separate rendering of the same decision, never a
    /// second decision.
    pub(crate) trust: Option<TrustPrompt>,
}

/// The TUI's startup workspace-trust modal: the same one decision the
/// plain REPL asks as a line prompt — trust this folder for the session,
/// name a different directory, or continue unbound — rendered inside the
/// interface after the splash paints. `draft` is `Some` once the `w <dir>`
/// line is being typed (`Some("")` right after `w`, before the first
/// directory character); a bad directory refuses inline (the modal stays,
/// with the error), never as a launch failure.
#[derive(Debug, Clone, Default)]
pub(crate) struct TrustPrompt {
    pub(crate) draft: Option<String>,
    pub(crate) error: Option<String>,
}
