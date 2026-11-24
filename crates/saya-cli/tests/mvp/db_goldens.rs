//! Database golden transcripts: today's database-side chat behaviour, frozen
//! before the database plugin refactor so every chunk can prove it changed
//! nothing.
//!
//! Each scenario drives the real REPL against the scripted provider harness
//! (the one `session_question_budget` uses) and a SQLite fixture in the
//! test's temp directory — no server, no podman. The record in
//! `db_goldens/golden/<scenario>.json` captures, per provider request, the
//! advertised tool names in order, the normalised request-body byte length,
//! and the message count; per tool call, its name, SQL if any, and outcome
//! class; and the user-visible state of every settled turn.
//!
//! A golden is green on today's code. When behaviour intentionally changes,
//! regenerate deliberately:
//!
//! ```text
//! SAYA_UPDATE_GOLDENS=1 cargo nextest run -p saya-cli --locked --test mvp \
//!     -E 'test(db_goldens)'
//! ```
//!
//! then review the diff like code — a changed golden is a behaviour change,
//! not a rubber stamp. Machine-specific text (temp roots, home paths, the
//! user name, the prompt-cache hash) is redacted by `normalise` before the
//! record is measured or compared.

// One harness instance for the whole test binary: `session_question_budget`
// owns the module, and the other rows reach it through this binding.
use crate::session_question_budget::harness;

mod classify;
mod frozen;
mod launch_presence;
#[path = "../support/launch_summary.rs"]
mod launch_summary;
mod launch_summary_tests;
mod normalise;
mod record;
mod record_b;
mod scenarios;
mod scenarios_b;
mod scenarios_budget;
mod setup;
mod setup_b;
