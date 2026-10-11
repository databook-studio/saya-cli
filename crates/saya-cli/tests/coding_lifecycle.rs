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

#[path = "coding_lifecycle/budget_regressions.rs"]
mod budget_regressions;
#[path = "coding_lifecycle/conflict_resolution.rs"]
mod conflict_resolution;
#[path = "coding_lifecycle/conflicts.rs"]
mod conflicts;
#[path = "coding_lifecycle/project_preferences.rs"]
mod project_preferences;
