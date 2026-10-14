#[path = "coding_runtime_contracts/guarded_applyback.rs"]
mod applyback;
#[path = "coding_runtime_contracts/applyback_preflight.rs"]
mod applyback_guard;
#[path = "coding_runtime_contracts/applyback_recovery.rs"]
mod applyback_recovery;
#[path = "coding_runtime_contracts/candidate_integration.rs"]
mod candidate_integration;
#[path = "coding_runtime_contracts/checkout_authority_boundaries.rs"]
mod checkout_authority_boundaries;
#[path = "coding_runtime_contracts/managed_worktree_creation.rs"]
mod checkout_creation;
#[path = "coding_runtime_contracts/checkout_write_authority.rs"]
mod checkout_write_authority;
#[path = "coding_runtime_contracts/child_effect_authorization.rs"]
mod child_effect_authorization;
#[path = "coding_runtime_contracts/child_loop_lifecycle.rs"]
mod child_loop_lifecycle;
#[path = "coding_runtime_contracts/confined_program_runner.rs"]
mod confined_program_runner;
#[path = "coding_runtime_contracts/process_confinement.rs"]
mod confinement;
#[path = "coding_runtime_contracts/worktree_delta_receipts.rs"]
mod delta;
#[path = "coding_runtime_contracts/child_environment_pinning.rs"]
mod environment_pinning;
#[cfg(all(unix, any(target_os = "macos", target_os = "linux")))]
#[path = "coding_runtime_contracts/execution_receipt_validation.rs"]
mod execution_receipts;
#[path = "coding_runtime_contracts/git_process_isolation.rs"]
mod git_processes;
#[path = "coding_runtime_contracts/review_conflict_evidence.rs"]
mod review_evidence;
#[path = "coding_runtime_contracts/snapshot_materialization.rs"]
mod snapshot_materialization;

#[path = "coding_runtime_contracts/skill_resource_contracts.rs"]
mod skill_resource_contracts;

#[path = "coding_runtime_contracts/safe_worktree_cleanup.rs"]
mod cleanup;

#[path = "coding_runtime_contracts/checkout_effect_reconciliation.rs"]
mod recovery;

#[path = "coding_runtime_contracts/checkout_effect_recovery.rs"]
mod checkout_effect_recovery;

#[path = "coding_runtime_contracts/mcp_transport_lifecycle.rs"]
mod mcp_transport;

#[path = "coding_runtime_contracts/lsp_process_lifecycle.rs"]
mod lsp_transport;

#[path = "coding_runtime_contracts/snapshot_restore.rs"]
mod snapshot_restore;
