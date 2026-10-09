//! Mutations for the run-spine tables.
//!
//! Every mutation is one honest operation: a transition that is not in the
//! state machine is `Conflict`, a record that does not exist is `NotFound`,
//! and a duplicate create is `Conflict` — the engine's single-writer lock
//! makes races rare, and the guarded `UPDATE ... WHERE status = ?` makes the
//! one that matters visible instead of silent.

mod run;
mod steps;

pub(crate) use run::{create_run, set_run_status, set_run_usage};
pub(crate) use steps::{set_step_usage, upsert_step};
