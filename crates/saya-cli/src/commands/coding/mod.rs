//! One native coding application owner shared by terminal and tool adapters.
mod approval;
mod approval_audit;
mod cancel_poll;
pub(crate) mod checks;
pub(crate) mod cli;
mod cli_mutations;
mod completion_view;
mod doctor;
mod doctor_budget;
mod doctor_extensions;
mod doctor_probe;
mod workspace_delete;
mod worktrees;
pub use cli::dispatch as dispatch_coding;
/// The doctor's one synthetic probe tool name. Re-exported under `cfg(test)`
/// so the allow-list drift test reads the production constant as its one
/// exempt name.
#[cfg(test)]
pub(crate) use doctor_probe::READINESS_PROBE_TOOL;
pub(crate) use worktrees::verify_root_checkout;
mod child_composition;
mod child_controls;
mod controls;
mod create;
pub(crate) mod database;
mod delegate;
mod delegate_batch;
mod follow_up;
mod helper_shape;
pub(crate) mod job_records;
/// The unix half of a job-record removal, kept apart so every unsafe block the
/// walk needs sits in one small module of its own.
#[cfg(unix)]
mod job_records_remove;
mod joined_accounting;
pub(crate) mod lane;
pub(crate) mod lifecycle;
pub(crate) mod limits;
mod origin;
pub use follow_up::{ChildFollowUpRequest, ChildFollowUpStarted};
mod diagnostic_notice;
mod diagnostics;
mod export;
mod failure;
mod finish_direct;
mod in_place_changes;
mod in_place_start;
mod in_place_status;
mod observations;
mod operations;
mod outcome;
mod parent;
/// The per-turn inputs a surface states when it opens a run, and the plugins
/// those turns may activate: the chat's composition states its own at
/// `run_prepare`.
pub(crate) use parent::{RunPlugins, TurnInputs};

/// The turn-per-message fixtures a chat turn's tests drive the real engine
/// with: the same in-place job and scripted provider the core's own turn tests
/// run on, so one turn of a session is measured on the engine itself.
#[cfg(all(test, unix))]
pub(crate) use parent::parent_turn_fixture;
#[cfg(test)]
pub(crate) use parent::parent_turn_provider;
mod prepare;
mod prepare_direct;
mod prepare_receipt;
mod progress_event;
mod progress_report;
mod project_context;
mod readiness;
mod readiness_cache;
mod readiness_direct;
mod readiness_generation;
mod usage;
pub use delegate::CodingTask;
mod assignment_added;
mod context;
mod continuation;
mod continuation_findings;
mod convergence;
mod host;
#[cfg(all(test, unix))]
#[path = "host_mode_tests.rs"]
mod host_mode_tests;
pub(crate) use host::mode::Launch;
mod host_root;
// A run's shape reads the folder's root identity and its lease before any job
// exists, and must never take the lease to ask about it: `chat::workspace_shape`
// is their one reader outside this module.
pub(crate) use host_root::resolve_in_place;
mod in_place_lease;
pub(crate) use in_place_lease::leased as folder_lease_held;
mod integrate;
mod live_runs;
// The owner of a lease this process holds, so a run's shape can tell this
// session's own folder from another owner's.
pub(crate) use live_runs::lease_owner as folder_lease_owner;
mod notes;
mod oneshot;
pub(crate) mod oneshot_cli;
mod prompts;
mod proposal;
mod provider_binding;
mod readiness_steps;
mod reconcile;
mod request;
mod resume_refusal;
mod rewind;
mod root_checkout;
mod root_direct;
pub(crate) mod root_mode;
mod session_parent;
mod setup;
mod task_flow;
mod task_history;
mod task_merge;
mod task_merge_record;
mod transcript;
pub use proposal::CodingApprovalRequest;
mod turn_stop;
pub(crate) use turn_stop::TurnStop;
mod review;
mod review_diff;
mod review_in_place;
pub use review::CodingReview;
mod extension_observer;
mod extensions;
mod known_commit;
mod known_mutation;
mod owner_failure;
mod post_edit;
mod runtime;
pub use approval::CodingAuthorization;
pub use host::CodingApplication;
pub(crate) use known_mutation::{KnownWorkspaceMutation, KnownWorkspaceMutationOperation};
pub use oneshot::{
    LaunchPlan, OneShotMode, OneShotRequest, OneShotResult, consent_implied,
    finish as oneshot_finish, generate_job_id, launch_plan, launch_write_scope, resolve_mode,
    start as oneshot_start,
};
pub(crate) use owner_failure::NativeOwnerFailureState;
pub use request::{CodingJobRequest, JobOrigin};
pub use runtime::NativeCodingJob;

pub(crate) use in_place_changes::job_changes;
pub(crate) use root_checkout::{RootChange, checkout_changes, read_root_checkout};

// The owner's one error, stated once for every surface that reports it.
mod error;
pub use error::CodingError;

#[cfg(all(test, unix))]
#[path = "failure_guidance_tests.rs"]
mod failure_guidance_tests;

mod workspace_search;

#[cfg(all(test, unix))]
#[path = "request_fixture.rs"]
mod request_fixture;

#[cfg(all(test, unix))]
#[path = "request_mode_tests.rs"]
mod request_mode_tests;

#[cfg(all(test, unix))]
#[path = "plan_gate_tests.rs"]
mod plan_gate_tests;

#[cfg(all(test, unix))]
#[path = "tools_allow_tests.rs"]
mod tools_allow_tests;

#[cfg(all(test, unix))]
#[path = "origin_tests.rs"]
mod origin_tests;

#[cfg(test)]
#[path = "in_place_lease_tests.rs"]
mod in_place_lease_tests;

#[cfg(all(test, unix))]
#[path = "root_checkout_tests.rs"]
mod root_checkout_tests;

#[cfg(all(test, unix))]
#[path = "root_verify_once_tests.rs"]
mod root_verify_once_tests;

#[cfg(all(test, unix))]
#[path = "delegate_direct_tests/fixture.rs"]
mod delegate_direct_fixture;

#[cfg(all(test, unix))]
#[path = "delegate_direct_tests.rs"]
mod delegate_direct_tests;

#[cfg(all(test, unix))]
#[path = "delegate_validation_tests.rs"]
mod delegate_validation_tests;

#[cfg(all(test, unix))]
#[path = "direct_root_fixture.rs"]
mod direct_root_fixture;

#[cfg(all(test, unix))]
#[path = "task_merge_summary_tests.rs"]
mod task_merge_summary_tests;

#[cfg(all(test, unix))]
#[path = "task_flow_concurrency_tests.rs"]
mod task_flow_concurrency_tests;

#[cfg(all(test, unix))]
#[path = "root_retained_tests.rs"]
mod root_retained_tests;

#[cfg(all(test, unix))]
#[path = "root_agentless_delete_tests.rs"]
mod root_agentless_delete_tests;

#[cfg(all(test, unix))]
#[path = "operator_tests.rs"]
mod operator_tests;

#[cfg(all(test, unix))]
#[path = "retired_text_tests.rs"]
mod retired_text_tests;
