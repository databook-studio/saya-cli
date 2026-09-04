//! Read-only verification probes built from submitted SQL.

mod fanout;
mod order_by;

#[cfg(test)]
mod fanout_tests;
#[cfg(test)]
mod order_by_tests;

pub use fanout::{FanoutProbe, fanout_probe};
pub use order_by::has_top_level_order_by;
