//! Full-screen terminal UI for the interactive session.
//!
//! A scrolling transcript region on top, a one-line status bar, a bordered
//! multi-line input box pinned to the bottom, and a slash-command popup that
//! opens the instant the line starts with '/' and filters as you type. Slash
//! commands and `/sql` execute and render into the transcript; live agent
//! streaming arrives in a later milestone. Non-TTY input uses a headless
//! executor, not this module. Rendering lives in `ui`.

pub(crate) mod agent;
mod application;
mod atref;
// pub(crate) so the agent layer's capture hook can reuse the same accounted
// budget as the direct `/sql` path — one source of truth, never a copy.
pub(crate) mod capture;
mod capture_agent;
#[cfg(test)]
mod capture_tests;
mod clipboard;
mod coding;
mod complete;
#[cfg(test)]
#[path = "continuation_acceptance_tests.rs"]
mod continuation_acceptance_tests;
mod dispatch;
mod dispatch_actions;
mod dispatch_contracts;
mod dispatch_investigation;
mod dispatch_runs;
mod exec;
pub(crate) mod export;
mod fuzzy;
mod history;
mod input;
mod keys;
pub(crate) mod loop_tick;
#[cfg(test)]
mod metrics_summary_tests;
pub(crate) mod replay;
mod replay_task;
mod run_panel;
mod run_panel_apply;
#[cfg(test)]
mod run_panel_snapshot_tests;
#[cfg(test)]
mod run_panel_tests;
mod run_worker;
#[cfg(test)]
mod scope_tests;
pub(crate) mod session;
mod session_save;
mod setup_apply;
mod sql_task;
mod startup_readiness;
mod stream_events;
mod table;
mod terminal;
// The `/setup` handoff boundary: the slash command wires it into the loop.
mod terminal_handoff;
#[cfg(test)]
mod terminal_handoff_tests;
pub(crate) mod transcript;
#[cfg(test)]
mod truncation_tests;
mod trust;
pub(super) mod types;
pub(crate) mod ui;
#[cfg(test)]
mod ui_snapshot_tests;
mod usage_footer;
mod usage_totals;
mod worker_permits;
#[cfg(test)]
mod worker_permits_tests;
pub(crate) mod wrap;

pub(crate) use session::{TrustOutcome, TuiSession, run};

// The new front reuses today's input buffer and completion instead of a
// second copy of either: the composer (`front::tui::view`) calls these, and
// one definition of the wrap and the popup's candidates serves both surfaces.
// The palette ranks names with the popup's own matcher, so no second fuzzy
// rule has to agree with this one.
#[allow(unused_imports)]
pub(crate) use complete::{Candidate, slash_candidates};
#[allow(unused_imports)]
pub(crate) use fuzzy::fuzzy_score;
#[allow(unused_imports)]
pub(crate) use input::InputBuffer;
#[allow(unused_imports)]
pub(crate) use ui::surface::markdown_spans_fenced;

// The new front paints its result tables from today's rendering rather than a
// second copy of it: the box table and its alignment, the wide-table scroller,
// the column fitting and the view state that scroller reads. A build of this
// module alone references none of it, so the re-exports read as unused.
#[allow(unused_imports)]
pub(crate) use table::{clip_table_block, format_table};
#[allow(unused_imports)]
pub(crate) use types::WideTableView;

// The new front enters and leaves the terminal through today's guard rather
// than a second raw-mode setup, and its panic hook restores through the same
// sequence the guard does: one teardown, three callers. A build of this module
// alone references neither, so the re-exports read as unused.
#[allow(unused_imports)]
pub(crate) use terminal::{TerminalGuard, restore_terminal};

// A copy tries the OS clipboard tool and then the terminal's own OSC 52, and
// the new front's `c` goes through this one attempt rather than a second copy
// of it (T14's rule for the terminal). A build of this module alone references
// neither, so the re-exports read as unused.
#[allow(unused_imports)]
pub(crate) use clipboard::{copy_to_native_clipboard, osc52_copy};

// The new front's `/sql` is today's direct query, not a second one: the bounded
// worker task (which goes through the read-only safety layer), the permit whose
// cap it holds, and the process-wide refusal when all four are taken. A build
// of this module alone references none of it, so the re-exports read as unused.
#[allow(unused_imports)]
pub(crate) use sql_task::{Followup, SqlTask, spawn as spawn_sql_task};
#[allow(unused_imports)]
pub(crate) use worker_permits::{CAP_REFUSAL, try_acquire_worker_permit};

// `/export` writes with today's two-phase export (encode under a ceiling, then
// publish atomically) rather than a second writer, so the bytes a new-surface
// export writes are the bytes today's writes.
#[allow(unused_imports)]
pub(crate) use export::write_result_for;
#[cfg(test)]
pub(crate) use export::write_result_overwrite;

// `/chart [kind] [path]` is parsed once, by the grammar the old TUI reads.
#[allow(unused_imports)]
pub(crate) use dispatch::parse_chart_args;

// The new surface's jobs drawer reads today's job plumbing rather than a second
// copy of it: the panel a job runs under and the messages its worker queues —
// its activity, and the calls it puts to the operator, which the drawer lists
// and the card answers. A build of this module alone references neither, so the
// re-exports read as unused.
#[allow(unused_imports)]
pub(crate) use coding::{CodingMessage, CodingPanel};

// `/investigation save` keeps the TUI's save-without-SQL fill — the latest
// query the surface shows, on the connection that ran it — as one function.
#[allow(unused_imports)]
pub(crate) use dispatch_investigation::fill_save_from_last_query;

// The query a bare `save` is filled from is today's `LastQuery`.
#[allow(unused_imports)]
pub(crate) use types::LastQuery;

// The trust question's answer is today's own type — the one the old modal's
// keys produce — so the new surface states the same answer, not a second
// spelling of it.
#[allow(unused_imports)]
pub(crate) use trust::{TrustResolution, trust_key};
