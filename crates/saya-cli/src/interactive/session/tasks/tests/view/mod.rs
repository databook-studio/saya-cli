//! Slice 5 red tests: the user's view of the task list — the `/tasks`
//! rendering the shared dispatcher hands both surfaces, and the `tasks:`
//! status segment the headless header and the TUI bar both read.
//!
//! The dispatcher owns the list's content (the model writes it, `/tasks
//! clear` empties it); this module renders it. The exact strings below are
//! the user-visible contract.

#[path = "dispatcher.rs"]
mod dispatcher;
#[path = "render.rs"]
mod render;
#[path = "status_segment.rs"]
mod status_segment;
