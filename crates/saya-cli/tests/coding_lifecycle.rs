//! Deterministic native entrypoint acceptance; no external agent executables.
#![cfg(unix)]
#[path = "coding_lifecycle/agent_model.rs"]
mod agent_model;
#[path = "coding_lifecycle/commands.rs"]
mod commands;
#[path = "coding_lifecycle/continuation.rs"]
mod continuation;
#[path = "coding_lifecycle/controls.rs"]
mod controls;
#[path = "coding_application/extensions.rs"]
mod extensions;
#[path = "coding_lifecycle/git_process_export.rs"]
mod git_process_export;
#[path = "coding_lifecycle/in_place.rs"]
mod in_place;
#[path = "coding_lifecycle/in_place_edit_time.rs"]
mod in_place_edit_time;
#[path = "coding_lifecycle/in_place_helpers.rs"]
mod in_place_helpers;
#[path = "coding_lifecycle/in_place_shell_changes.rs"]
mod in_place_shell_changes;
#[path = "coding_lifecycle/in_place_show.rs"]
mod in_place_show;
#[path = "coding_lifecycle/job_outcome.rs"]
mod job_outcome;
#[path = "coding_lifecycle/job_token_ceiling.rs"]
mod job_token_ceiling;
#[path = "coding_lifecycle/managed_fallback.rs"]
mod managed_fallback;
#[path = "coding_lifecycle/native.rs"]
mod native;
#[path = "coding_lifecycle/oneshot.rs"]
mod oneshot;
#[path = "coding_lifecycle/post_edit_workspace.rs"]
mod post_edit_workspace;
#[path = "coding_lifecycle/provider.rs"]
mod provider;
#[path = "coding_lifecycle/read_only.rs"]
mod read_only;
#[path = "coding_lifecycle/root_cancel.rs"]
mod root_cancel;
#[path = "coding_lifecycle/start_consent.rs"]
mod start_consent;
#[path = "coding_application/support.rs"]
#[allow(dead_code)] // shared test harness: this file uses only part of it
mod support;
#[path = "coding_lifecycle/tool_parity.rs"]
mod tool_parity;
#[path = "coding_lifecycle/tools_allow.rs"]
mod tools_allow;
#[path = "coding_lifecycle/uncapped_beyond_default.rs"]
mod uncapped_beyond_default;
#[path = "coding_lifecycle/uncapped_mode.rs"]
mod uncapped_mode;
#[path = "coding_lifecycle/usage_report.rs"]
mod usage_report;
#[path = "coding_lifecycle/usage_stream_cut.rs"]
mod usage_stream_cut;
#[path = "coding_lifecycle/usage_unknown_cut.rs"]
mod usage_unknown_cut;
#[path = "coding_lifecycle/usage_unknown_exhausted.rs"]
mod usage_unknown_exhausted;
#[path = "coding_lifecycle/workspace_retry.rs"]
mod workspace_retry;
#[path = "coding_lifecycle/write_scope.rs"]
mod write_scope;

#[path = "coding_lifecycle/budget_regressions.rs"]
mod budget_regressions;
#[path = "coding_lifecycle/prepare_candidate.rs"]
mod prepare_candidate;
#[path = "coding_lifecycle/project_preferences.rs"]
mod project_preferences;
#[path = "coding_lifecycle/reload_settings.rs"]
mod reload_settings;

#[path = "coding_lifecycle/workspace_delete.rs"]
mod workspace_delete;

#[path = "coding_lifecycle/workspace_delete_refusals.rs"]
mod workspace_delete_refusals;

#[path = "coding_lifecycle/repair_controls.rs"]
mod repair_controls;

#[path = "coding_lifecycle/convergence.rs"]
mod convergence;
#[path = "coding_lifecycle/default_mode.rs"]
mod default_mode;
#[path = "coding_lifecycle/delegate_cap.rs"]
mod delegate_cap;
#[path = "coding_lifecycle/direct_checklist_delta.rs"]
mod direct_checklist_delta;
#[path = "coding_lifecycle/direct_finish.rs"]
mod direct_finish;
#[path = "coding_lifecycle/direct_follow_up.rs"]
mod direct_follow_up;
#[path = "coding_lifecycle/direct_implicit_join.rs"]
mod direct_implicit_join;
#[path = "coding_lifecycle/direct_prepare.rs"]
mod direct_prepare;
#[path = "coding_lifecycle/direct_progress.rs"]
mod direct_progress;
#[path = "coding_lifecycle/direct_readiness_facts.rs"]
mod direct_readiness_facts;
#[path = "coding_lifecycle/in_place_default.rs"]
mod in_place_default;
#[path = "coding_lifecycle/lifecycle_timing.rs"]
mod lifecycle_timing;
#[path = "coding_lifecycle/progress_guards.rs"]
mod progress_guards;

#[path = "coding_lifecycle/direct_rewind.rs"]
mod direct_rewind;
#[path = "coding_lifecycle/direct_rewind_model.rs"]
mod direct_rewind_model;
#[path = "coding_lifecycle/direct_status_cache.rs"]
mod direct_status_cache;
#[path = "coding_lifecycle/direct_task.rs"]
mod direct_task;
#[path = "coding_lifecycle/direct_task_join.rs"]
mod direct_task_join;
#[path = "coding_lifecycle/operator_request.rs"]
mod operator_request;
