//! The session grant commands' one behaviour: `/allow` seeds the session's
//! grant store, `/grants` lists it. The headless loop and the TUI dispatch
//! both call these, so the two surfaces render one operation — the words
//! the user typed are the record, and the parser is the grammar's only
//! authority.

mod allow;
mod journal;
mod launch;
mod listing;
mod run_seed;

pub(crate) use allow::allow;
pub(crate) use journal::{journal_warning, record_prompt_answer};
pub(crate) use launch::seed_launch_allow;
pub(crate) use listing::listing;
pub(crate) use run_seed::{RunSeed, run_seed, seed_message};
