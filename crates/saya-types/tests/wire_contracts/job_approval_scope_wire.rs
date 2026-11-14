use std::num::NonZeroU64;

use saya_types::{
    ApprovalDisposition, ApprovalRequestContext, ApprovalRequestV1, ApprovalScope,
    JobApprovalAction, JobId, ManagedCheckoutId, Sha256Digest,
};

const SHA_POLICY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_PROPOSAL: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const JOB_SCOPE_WIRE: &str = r#"{"operation":"job_operation","action":"resume","proposal_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}"#;
const MALFORMED_DIGEST_WIRE: &str = r#"{"operation":"job_operation","action":"resume","proposal_sha256":"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"}"#;

fn positive(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn context() -> ApprovalRequestContext {
    ApprovalRequestContext::new(Sha256Digest::parse(SHA_POLICY).unwrap(), 0, 0, 5000).unwrap()
}

#[test]
fn job_operation_scope_round_trips_with_its_exact_wire_form() {
    let scope = ApprovalScope::job_operation(
        JobApprovalAction::Resume,
        Sha256Digest::parse(SHA_PROPOSAL).unwrap(),
    );
    assert_eq!(serde_json::to_string(&scope).unwrap(), JOB_SCOPE_WIRE);
    assert_eq!(
        serde_json::from_str::<ApprovalScope>(JOB_SCOPE_WIRE).unwrap(),
        scope
    );
    let error = serde_json::from_str::<ApprovalScope>(MALFORMED_DIGEST_WIRE).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("digest must be 64 lowercase hexadecimal characters")
    );
}

#[test]
fn an_approval_request_can_carry_a_job_operation_scope() {
    let job_scope = ApprovalScope::job_operation(
        JobApprovalAction::Start,
        Sha256Digest::parse(SHA_PROPOSAL).unwrap(),
    );
    let request = ApprovalRequestV1::new_for_coordinator(
        JobId::parse("job-7").unwrap(),
        positive(19),
        context(),
        job_scope.clone(),
    )
    .unwrap();
    let decoded = ApprovalRequestV1::from_json_bytes(&request.to_json_bytes().unwrap()).unwrap();
    assert_eq!(decoded.scope(), &job_scope);

    let effect_scope = ApprovalScope::workspace_write(
        ManagedCheckoutId::parse("checkout-1").unwrap(),
        "src/lib.rs",
    )
    .unwrap();
    let effect_request = ApprovalRequestV1::new_for_coordinator(
        JobId::parse("job-7").unwrap(),
        positive(20),
        context(),
        effect_scope.clone(),
    )
    .unwrap();
    let effect_decoded =
        ApprovalRequestV1::from_json_bytes(&effect_request.to_json_bytes().unwrap()).unwrap();
    assert_eq!(effect_decoded.scope(), &effect_scope);
}

#[test]
fn granted_is_a_frozen_audit_disposition() {
    assert_eq!(
        serde_json::to_string(&ApprovalDisposition::Granted).unwrap(),
        r#"{"disposition":"granted"}"#
    );
    assert_eq!(
        serde_json::from_str::<ApprovalDisposition>(r#"{"disposition":"granted"}"#).unwrap(),
        ApprovalDisposition::Granted
    );
    assert!(serde_json::from_str::<ApprovalDisposition>(r#"{"disposition":"accepted"}"#).is_err());
}
