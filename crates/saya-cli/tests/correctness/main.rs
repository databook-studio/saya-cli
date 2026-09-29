//! Deterministic correctness scenarios (B3d): the scripted-provider harness
//! drives the real `saya ask` binary against the real demo fixture, and every
//! scenario asserts on the rows saya returned — not merely that a result came
//! back. See each scenario module for the trap it pins.

mod active_ambiguity;
mod common;
mod confirmed_claim;
mod duplicate_join;
mod time_column;
