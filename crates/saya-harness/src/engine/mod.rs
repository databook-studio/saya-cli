//! The run engine.
//!
//! Only the state machine exists so far: a pure transition table over the run
//! lifecycle, with no I/O and no async, so the legal shape of a run can be
//! settled and tested before anything drives it. The episode driver, resume and
//! the event sink land on top of this.

mod state;

pub use state::{RunState, RunTransition, RunTransitionError, transition};
