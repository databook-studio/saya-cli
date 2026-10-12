use super::*;
use saya_harness::coding::policy::{AuthorityError, RootDelegationInput, RootDelegationScope};
use saya_types::{
    AgentId, Capabilities, InvocationId, JobId, RepositoryId, Sha256Digest, SnapshotId,
};

fn delegation_input(job: &str, agent: &str, invocation: &str) -> RootDelegationInput {
    RootDelegationInput {
        job_id: JobId::parse(job).unwrap(),
        agent_id: AgentId::parse(agent).unwrap(),
        invocation_id: InvocationId::parse(invocation).unwrap(),
        repository_id: RepositoryId::parse("repo").unwrap(),
        snapshot_id: SnapshotId::parse("snapshot").unwrap(),
        managed_checkout_id: None,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 0,
        write_paths: vec![],
        capabilities: Capabilities::default(),
        database_profiles: vec![],
    }
}

#[tokio::test]
async fn child_issuance_approval_refuses_a_different_childs_request() {
    let input_a = delegation_input("job", "agent-a", "invocation-a");
    let decider_for_a = ChildIssuanceApproval {
        expected: input_a.approval_request(),
    };
    let input_b = delegation_input("job", "agent-b", "invocation-b");
    assert!(matches!(
        RootDelegationScope::approve(input_b, &decider_for_a).await,
        Err(AuthorityError::ApprovalDenied)
    ));
    let input_a_again = delegation_input("job", "agent-a", "invocation-a");
    assert!(
        RootDelegationScope::approve(input_a_again, &decider_for_a)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn child_issuance_approval_refuses_another_jobs_request() {
    let input_a = delegation_input("job-a", "agent", "invocation");
    let decider_for_a = ChildIssuanceApproval {
        expected: input_a.approval_request(),
    };
    let input_b = delegation_input("job-b", "agent", "invocation");
    assert!(matches!(
        RootDelegationScope::approve(input_b, &decider_for_a).await,
        Err(AuthorityError::ApprovalDenied)
    ));
    let input_a_again = delegation_input("job-a", "agent", "invocation");
    assert!(
        RootDelegationScope::approve(input_a_again, &decider_for_a)
            .await
            .is_ok()
    );
}

#[test]
fn a_missing_approval_names_the_action_that_supplies_it() {
    let text = CodingError::ApprovalRequired.to_string();
    assert!(
        text.contains("--approve") && text.contains("/coding approve"),
        "scripts and interactive sessions each get their own action: {text}"
    );
}
