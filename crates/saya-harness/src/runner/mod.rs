//! The runner capability's sandbox half (M5-3): the OS-level confinement the
//! `run_program` tool (M5-4) will spawn children under. This module owns the
//! policy, the fail-closed startup probe, and the per-platform spawn
//! mechanics; the tool itself is deliberately not wired here.

pub mod sandbox;
