//! R04a settlement-truth tests: estimates surface separately and are never enforced.

#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "budgeted_child_scheduler/fixture.rs"]
mod fixture;

#[path = "usage_estimate_reporting/estimates.rs"]
mod estimates;

#[path = "usage_estimate_reporting/restart.rs"]
mod restart;
