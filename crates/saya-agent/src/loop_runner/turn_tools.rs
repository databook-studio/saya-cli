//! Executes the tool calls of one assistant turn — concurrently only when
//! every call is valid, auto-runnable, and explicitly independent. A mixed
//! concurrent/serial batch is conservatively serial, as are prompted calls.
//! The caller trims the intra-loop context budget after either path returns.

mod batch;
mod runner;
mod serial;
mod state;

pub(super) use runner::run_turn_tools;
