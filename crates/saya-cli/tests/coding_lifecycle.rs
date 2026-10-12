//! Deterministic native entrypoint acceptance; no external agent executables.
#![cfg(unix)]
#[path = "coding_lifecycle/commands.rs"]
mod commands;
#[path = "coding_lifecycle/continuation.rs"]
mod continuation;
#[path = "coding_lifecycle/controls.rs"]
mod controls;
#[path = "coding_lifecycle/git_process_export.rs"]
mod git_process_export;
#[path = "coding_lifecycle/native.rs"]
mod native;
#[path = "coding_lifecycle/provider.rs"]
mod provider;
#[path = "coding_lifecycle/root_cancel.rs"]
mod root_cancel;
#[path = "primary_coding/support.rs"]
#[allow(dead_code)] // shared test harness: this file uses only part of it
mod support;

#[path = "coding_lifecycle/budget_regressions.rs"]
mod budget_regressions;
#[path = "coding_lifecycle/conflict_resolution.rs"]
mod conflict_resolution;
#[path = "coding_lifecycle/conflicts.rs"]
mod conflicts;
#[path = "coding_lifecycle/project_preferences.rs"]
mod project_preferences;
#[path = "coding_lifecycle/reload_settings.rs"]
mod reload_settings;
