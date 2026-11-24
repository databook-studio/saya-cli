//! The jobs the drawer's tests list: one in each of `02` §8's three states,
//! and the drawer over them.
//!
//! The fixture is the spec's own mock-up read back: `● fix-flaky running 3m · 2
//! helpers`, `⏸ docs-pass paused: no progress`, `✓ bump-deps finished · 4 files
//! changed`. A pane reworded into agreeing with itself would still fail.

use super::jobs::{Drawer, Job, State};

/// `02` §8's first row: a job that is working, with its age and its helpers.
pub(crate) fn running() -> Job {
    Job {
        id: String::from("fix-flaky"),
        state: State::Running,
        age: String::from("3m"),
        helpers: 2,
        summary: String::from("query demo · 3 rows"),
        call: None,
    }
}

/// `02` §8's second row: a job that stopped itself, and why.
pub(crate) fn paused() -> Job {
    Job {
        id: String::from("docs-pass"),
        state: State::Paused,
        summary: String::from("no progress"),
        ..Job::default()
    }
}

/// `02` §8's third row: a job that finished, and what it ended with.
pub(crate) fn finished() -> Job {
    Job {
        id: String::from("bump-deps"),
        state: State::Finished,
        summary: String::from("4 files changed"),
        ..Job::default()
    }
}

/// The three rows, in the order the drawer lists them.
pub(crate) fn jobs() -> Vec<Job> {
    vec![running(), paused(), finished()]
}

/// The drawer over them, the first row selected.
pub(crate) fn drawer() -> Drawer {
    Drawer::new(jobs())
}
