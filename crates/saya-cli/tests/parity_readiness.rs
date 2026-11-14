//! Actual readiness collection; no provider or managed runtime execution.
#![cfg(unix)]
#[path = "parity_readiness/doctor.rs"]
mod doctor;
#[allow(dead_code)]
#[path = "coding_application/support.rs"]
mod native;
#[path = "parity_readiness/support.rs"]
mod support;
