//! The leaves X1 kept for the new surface.
//!
//! This module is no longer a surface: T19 switched `saya` to `front/tui`, and
//! X1 deleted the old frame — its app state, its dispatch, its event loop, its
//! keys, its chrome, its overlays and its snapshots. What is left here is what
//! `front/tui` imports rather than copies: the composer's buffer and
//! completion, the wrap and fuzzy rules, the input history, the tables, the
//! markdown renderer, the theme's hues and detection, the clipboard attempt,
//! the terminal guard and its `/setup` handoff, the two-phase export, the
//! capture's evidence, the trust answer, and the job plumbing.
//!
//! Everything below is re-exported here so the new surface names one module.

pub(crate) mod capture;
mod clipboard;
mod coding;
mod complete;
mod exec;
pub(crate) mod export;
mod fuzzy;
mod history;
mod input;
pub(crate) mod replay;
mod sql_task;
mod table;
mod terminal;
// The `/setup` handoff boundary: the surface wires it in as a command.
mod terminal_handoff;
#[cfg(test)]
mod terminal_handoff_tests;
mod trust;
pub(crate) mod ui;
mod worker_permits;
#[cfg(test)]
mod worker_permits_tests;
pub(crate) mod wrap;

/// What both surfaces say to a quit while a job of the session still runs: a
/// job is a worker this session owns, and a quit would orphan it — so the
/// operator cancels it, or waits for it to settle.
pub(crate) const JOB_RUNNING: &str =
    "A job is still running. Use `/job cancel <job-id>`, then wait for settlement before quitting.";

// The new front reuses today's input buffer and completion instead of a
// second copy of either: the composer (`front::tui::view`) calls these, and
// one definition of the wrap and the popup's candidates serves both surfaces.
// The palette ranks names with the popup's own matcher, so no second fuzzy
// rule has to agree with this one.
#[allow(unused_imports)]
pub(crate) use complete::{Candidate, slash_candidates};
#[allow(unused_imports)]
pub(crate) use fuzzy::fuzzy_score;
// The new front's composer keeps today's input history — the ring, its
// `input_history` file and its `--param` redaction — rather than a second copy
// of it: `Up`/`Down` recall what `push` recorded, and `Ctrl+R` searches it.
#[allow(unused_imports)]
pub(crate) use history::History;
#[allow(unused_imports)]
pub(crate) use input::InputBuffer;
#[allow(unused_imports)]
pub(crate) use ui::surface::markdown_spans_fenced;

// The new front paints its result tables from today's rendering rather than a
// second copy of it: the box table and its alignment, the wide-table scroller,
// the column fitting and the view state that scroller reads.
#[allow(unused_imports)]
pub(crate) use table::{WideTableView, clip_table_block, format_table};

// The new front enters and leaves the terminal through today's guard rather
// than a second raw-mode setup, and its panic hook restores through the same
// sequence the guard does: one teardown, three callers.
#[allow(unused_imports)]
pub(crate) use terminal::{TerminalGuard, restore_terminal};

// `/setup`'s terminal handoff is the leaf this release keeps: the new front
// releases the alternate screen, runs the same guided wizard in the normal one
// and always comes back through it, rather than a second copy of the boundary.
#[allow(unused_imports)]
pub(crate) use terminal_handoff::{SetupHandoff, TerminalModes, run_guided_setup};

// A copy tries the OS clipboard tool and then the terminal's own OSC 52, and
// the new front's `c` goes through this one attempt rather than a second copy
// of it (T14's rule for the terminal).
#[allow(unused_imports)]
pub(crate) use clipboard::{copy_to_native_clipboard, osc52_copy};

// The new front's `/sql` is today's direct query, not a second one: the bounded
// worker task (which goes through the read-only safety layer), the permit whose
// cap it holds, the process-wide refusal when all four are taken, and the last
// query a bare `save` is filled from.
#[allow(unused_imports)]
pub(crate) use sql_task::{LastQuery, SqlTask, spawn as spawn_sql_task};
#[allow(unused_imports)]
pub(crate) use worker_permits::{CAP_REFUSAL, try_acquire_worker_permit};

// `/export` writes with today's two-phase export (encode under a ceiling, then
// publish atomically) rather than a second writer, so the bytes a new-surface
// export writes are the bytes today's writes.
#[allow(unused_imports)]
pub(crate) use export::write_result_for;
#[cfg(test)]
pub(crate) use export::write_result_overwrite;

// The new surface's jobs drawer reads today's job plumbing rather than a second
// copy of it: the panel a job runs under and the messages its worker queues —
// its activity, and the calls it puts to the operator, which the drawer lists
// and the card answers — plus the look that reads them and the worker itself,
// which is how `/start` opens a job here.
#[allow(unused_imports)]
pub(crate) use coding::{
    CONTROL_FULL, CodingMessage, CodingPanel, NO_PENDING, Said, WORKER_GONE, spawn_job,
    started_line,
};

// The trust question's answer is today's own type — the spelling the old modal's
// keys produced — so the new surface states the same answer, not a second
// reading of it.
#[allow(unused_imports)]
pub(crate) use trust::TrustResolution;

// The new surface's rows and drawers state when something happened in today's
// words ("3m ago"), so both surfaces read one clock.
#[allow(unused_imports)]
pub(crate) use replay::relative_time;

// The ground a launch paints with: `[ui] theme` and `--theme` resolve here,
// over the terminal's own answer when the choice is `auto`.
#[allow(unused_imports)]
pub(crate) use ui::theme::{
    Theme, decide_public, probe_terminal_theme, resolve_startup_theme, set_color_enabled, set_theme,
};

// The starter questions the empty state offers are built from schema metadata
// by one function, read here by the new surface's welcome pane.
#[allow(unused_imports)]
pub(crate) use ui::surface::starter_questions;
