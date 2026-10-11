//! Deterministic native entrypoint acceptance; no external agent executables.
#![cfg(unix)]
#[path = "coding_lifecycle/commands.rs"]
mod commands;
#[path = "coding_lifecycle/native.rs"]
mod native;
#[path = "coding_lifecycle/provider.rs"]
mod provider;
#[path = "primary_coding/support.rs"]
mod support;
