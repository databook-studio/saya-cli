//! Bounded, journaled observation of git subprocess lifecycles.

mod observer;
mod receipt;
mod snapshot;

pub(super) use observer::{GitProcessObserver, Observation};
pub use receipt::{GitProcessEffect, GitProcessOutcome, GitProcessReceipt};
pub use snapshot::GitProcessSnapshot;
