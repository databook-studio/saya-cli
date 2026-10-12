pub(crate) const BUDGET_RESERVED: &str = r#"{
  "version": 1, "sequence": 5, "job_id": "job-1", "agent_id": "agent-1",
  "invocation_id": "inv-1", "event": {"type": "budget_reserved", "reservation": {
    "version": 1, "job_id": "job-1", "reservation_sequence": 5,
    "agent_id": "agent-1", "invocation_id": "inv-1", "charges": [{
      "dimension": "logical_requests", "endpoint": null,
      "price_metadata_sha256": null, "reserved": 1
    }]
  }}
}"#;

pub(crate) const BUDGET_SETTLED: &str = r#"{
  "version": 1, "sequence": 6, "job_id": "job-1", "event": {
    "type": "budget_settled", "settlement": {
      "version": 1, "job_id": "job-1", "reservation_sequence": 5,
      "settlement_sequence": 6, "effect_state": "uncertain", "observations": [{
        "dimension": "logical_requests", "endpoint": null,
        "price_metadata_sha256": null, "value": {"status": "unknown"}
      }]
    }
  }
}"#;

pub(crate) const BUDGET_RECONCILED: &str = r#"{
  "version": 1, "sequence": 14, "job_id": "job-1", "event": {
    "type": "budget_reconciled", "reservation_sequence": 5,
    "note_sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
  }
}"#;

pub(crate) const APPROVAL_REQUESTED: &str = r#"{
  "version": 1, "sequence": 7, "job_id": "job-1", "event": {
    "type": "approval_requested", "request": {
      "version": 1, "job_id": "job-1", "agent_id": null, "invocation_id": null,
      "request_sequence": 7,
      "policy_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "revocation_epoch": 0, "requested_at_unix_ms": 0, "expires_at_unix_ms": 5000,
      "scope": {"operation": "workspace_write", "managed_checkout_id": "checkout-1",
        "relative_path": "src/lib.rs"}
    }
  }
}"#;

pub(crate) const APPROVAL_CLOSED_CONSUMED: &str = r#"{
  "version": 1, "sequence": 9, "job_id": "job-1", "event": {
    "type": "approval_closed", "request_sequence": 7,
    "disposition": {"disposition": "consumed", "reservation_sequence": 8}
  }
}"#;

pub(crate) const CHECKOUT_CREATE_INTENT: &str = r#"{
  "version": 1, "sequence": 11, "job_id": "job-1", "agent_id": "agent-1",
  "event": {"type": "checkout_create_intent", "checkout": {
    "version": 1, "job_id": "job-1", "agent_id": "agent-1",
    "managed_checkout_id": "checkout-1", "repository_id": "repo-shared-1",
    "common_dir_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "managed_path": "/Users/saya/state/checkouts/checkout-1",
    "base_commit": "1111111111111111111111111111111111111111",
    "snapshot_id": "snapshot-1",
    "snapshot_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    "created_by_sequence": 11, "lifecycle": "creating"
  }, "operation": {
    "version": 1, "job_id": "job-1", "operation_sequence": 11,
    "managed_checkout_id": "checkout-1", "action": "create_checkout", "state": "intent"
  }}
}"#;

pub(crate) const CHECKOUT_OPERATION_INTENT: &str = r#"{
  "version": 1, "sequence": 12, "job_id": "job-1", "agent_id": "agent-1",
  "event": {"type": "checkout_operation_intent", "operation": {
    "version": 1, "job_id": "job-1", "operation_sequence": 12,
    "managed_checkout_id": "checkout-1", "action": "remove_checkout", "state": "intent"
  }}
}"#;

pub(crate) const CHECKOUT_OPERATION_OUTCOME: &str = r#"{
  "version": 1, "sequence": 13, "job_id": "job-1", "agent_id": "agent-1",
  "event": {"type": "checkout_operation_outcome", "operation": {
    "version": 1, "job_id": "job-1", "operation_sequence": 12,
    "managed_checkout_id": "checkout-1", "action": "remove_checkout", "state": "uncertain"
  }}
}"#;

pub(super) const NOTE_SHA: &str =
    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
