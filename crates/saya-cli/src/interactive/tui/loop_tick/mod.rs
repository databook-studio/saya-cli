//! One event-loop tick, split by concern: events, mouse, clipboard, workers.

pub(crate) mod busy;
pub(crate) mod clipboard;
pub(crate) mod events;
pub(crate) mod mouse;
pub(crate) mod pending;
pub(crate) mod resume;
pub(crate) mod workers;

pub(crate) use busy::tick_busy;
pub(crate) use clipboard::tick_clipboard;
pub(crate) use events::tick_events;
pub(crate) use mouse::{MouseCapture, tick_mouse_capture};
pub(crate) use pending::tick_pending;
pub(crate) use resume::tick_resume;
pub(crate) use workers::tick_workers;
