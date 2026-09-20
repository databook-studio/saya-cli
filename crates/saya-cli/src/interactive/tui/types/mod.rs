//! Shared TUI state types used by the event loop, application logic, and renderer.

pub(crate) mod app;
pub(crate) mod overlays;
pub(crate) mod request;
pub(crate) mod tasks;
pub(crate) mod usage;

pub(crate) use app::App;
pub(crate) use overlays::{
    Menu, OverlayState, Picker, PickerEntry, SearchKind, SearchOverlay, TrustPrompt,
};
pub(crate) use request::{MAX_INPUT_ROWS, PendingApproval, RequestState};
pub(crate) use tasks::{ClipboardCopy, CompactOutcome, LastQuery, SessionSave, WideTableView};
pub(crate) use usage::SessionUsage;

#[cfg(test)]
#[path = "../types_tests.rs"]
mod types_tests;
