use async_trait::async_trait;
use saya_agent::CancellationToken;
use saya_agent::{AllowReadOnlyApproval, ApprovalDecider, ToolDefinition};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_types::{CodingBudgetMode, JobId};
use serde_json::json;

#[tokio::test]
#[cfg(unix)]
async fn coding_grant_is_not_read_only_and_cannot_be_reconstructed_from_prose() {
    let request = CodingJobRequest::new(
        JobId::parse("job-approval").unwrap(),
        "change one file".into(),
        vec!["src/lib.rs".into()],
    )
    .unwrap();
    let fixture = super::support::Fixture::new("grant-binding").await;
    let proposal = fixture.application.propose(&request).await.unwrap();
    assert_eq!(proposal.request()["budget_mode"], "bounded");
    let authorization = CodingAuthorization::scoped(&proposal).unwrap();
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

/// The start output states the sensitive paths the snapshot leaves out, and
/// the total count, so consent covers the omission without naming bytes.
#[tokio::test]
#[cfg(unix)]
async fn a_git_proposal_lists_the_sensitive_paths_the_snapshot_omits() {
    let fixture = super::support::Fixture::new("proposal-omitted-sensitive").await;
    std::fs::write(
        fixture.repository.join("secrets_manager.py"),
        b"print('x')\n",
    )
    .unwrap();
    std::fs::write(fixture.repository.join(".netrc"), b"machine example\n").unwrap();
    std::fs::write(fixture.repository.join(".env.local"), b"TOKEN=x\n").unwrap();
    let request = CodingJobRequest::new(
        JobId::parse("job-omitted-sensitive").unwrap(),
        "change one file".into(),
        vec!["tracked.txt".into()],
    )
    .unwrap();
    let proposal = fixture.application.propose(&request).await.unwrap();
    let mut omitted = proposal.request()["omitted_sensitive"]
        .as_array()
        .expect("the proposal lists the omitted sensitive paths")
        .iter()
        .map(|path| path.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    omitted.sort();
    assert_eq!(omitted, vec![".env.local", ".netrc"]);
    assert_eq!(proposal.request()["omitted_sensitive_count"], json!(2));
    assert!(
        !omitted.iter().any(|path| path == "secrets_manager.py"),
        "an ordinary source name is never reported as sensitive"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn uncapped_mode_is_bound_in_start_and_resume_approval_requests() {
    let fixture = super::support::Fixture::new("uncapped-approval-mode").await;
    let mut runtime = fixture.runtime.clone();
    runtime.resolved.limits.job.budget_mode = CodingBudgetMode::Uncapped;
    let application = CodingApplication::open(
        runtime,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding-uncapped"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let job_id = JobId::parse("job-uncapped-approval").unwrap();
    let request = CodingJobRequest::new(
        job_id.clone(),
        "change one file".into(),
        vec!["tracked.txt".into()],
    )
    .unwrap();
    let proposal = application.propose(&request).await.unwrap();
    assert_eq!(proposal.request()["budget_mode"], "uncapped");
    let authorization = CodingAuthorization::scoped(&proposal).unwrap();
    let job = application
        .create(request, &authorization, CancellationToken::new())
        .await
        .unwrap();
    let resume = application.propose_resume(&job_id).await.unwrap();
    assert_eq!(resume.request()["operation"], "resume");
    assert_eq!(resume.request()["budget_mode"], "uncapped");
    drop(job);
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
        capabilities: Capabilities::default(),
        database_profiles: vec![],
    };
    assert!(matches!(
        RootDelegationScope::approve(input, &Deny).await,
        Err(AuthorityError::ApprovalDenied)
    ));
}

#[test]
fn selected_database_names_are_child_usable_before_scope_approval() {
    for (name, accepted) in [
        ("analytics-team".to_owned(), true),
        ("a".repeat(saya_types::MAX_NAME_CHARS), true),
        ("a".repeat(saya_types::MAX_NAME_CHARS + 1), false),
        ("analytics team".to_owned(), false),
        ("analytics\u{2003}team".to_owned(), false),
    ] {
        let request = CodingJobRequest::new(
            JobId::parse("name-bounds").unwrap(),
            "Inspect the approved database".into(),
            vec![],
        )
        .unwrap();
        assert_eq!(
            request.with_database_profiles(vec![name.clone()]).is_ok(),
            accepted,
            "a selected name must be usable by a child before any job is created: {name:?}"
        );
    }
}
