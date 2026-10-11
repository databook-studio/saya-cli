use async_trait::async_trait;
use saya_agent::{AllowReadOnlyApproval, ApprovalDecider, ToolDefinition};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use serde_json::json;

#[tokio::test]
#[cfg(unix)]
async fn coding_grant_is_not_read_only_and_cannot_be_reconstructed_from_prose() {
    let request = CodingJobRequest::new(
        JobId::parse("job-approval").unwrap(),
        "change one file".into(),
        vec!["src/lib.rs".into()],
        vec!["cargo".into()],
    )
    .unwrap();
    let fixture = super::support::Fixture::new("grant-binding").await;
    let authorization =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let definition = CodingAuthorization::definition();
    assert!(
        !AllowReadOnlyApproval
            .approve(&definition, &authorization.request())
            .await
    );
    let mut changed = authorization.request();
    changed["job_id"] = json!("other-job");
    assert!(!authorization.approve(&definition, &changed).await);
    assert!(
        authorization
            .approve(&definition, &authorization.request())
            .await
    );
    assert!(
        !authorization
            .approve(&definition, &authorization.request())
            .await,
        "scoped preauthorization is single-use"
    );
}

struct Deny;
#[async_trait]
impl ApprovalDecider for Deny {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        false
    }
}

#[tokio::test]
async fn root_scope_requires_fresh_effect_approval() {
    use saya_harness::coding::policy::{AuthorityError, RootDelegationInput, RootDelegationScope};
    use saya_types::{AgentId, Capabilities, InvocationId, RepositoryId, Sha256Digest, SnapshotId};
    let input = RootDelegationInput {
        job_id: JobId::parse("job").unwrap(),
        agent_id: AgentId::parse("agent").unwrap(),
        invocation_id: InvocationId::parse("invocation").unwrap(),
        repository_id: RepositoryId::parse("repo").unwrap(),
        snapshot_id: SnapshotId::parse("snapshot").unwrap(),
        managed_checkout_id: None,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 0,
        write_paths: vec![],
        capabilities: Capabilities::default(),
        database_profiles: vec![],
    };
    assert!(matches!(
        RootDelegationScope::approve(input, &Deny).await,
        Err(AuthorityError::ApprovalDenied)
    ));
}
