//! Read-only verification probes built from submitted SQL.

mod fanout;

#[cfg(test)]
mod fanout_tests;

pub use fanout::{FanoutProbe, fanout_probe};
