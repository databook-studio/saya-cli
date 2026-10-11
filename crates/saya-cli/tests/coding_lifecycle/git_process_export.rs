use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;

#[tokio::test]
async fn export_projects_pre_job_local_git_evidence_without_claiming_global_completeness() {
    let fixture = Fixture::new("git-process-export").await;
    let request = CodingJobRequest::new(
        JobId::parse("git-process-export").unwrap(),
        "export".into(),
        vec![],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    fixture
        .application
        .create(request.clone(), &approval, CancellationToken::new())
        .await
        .unwrap();
    let export = fixture
        .application
        .export(&request.job_id(), None, None)
        .await
        .unwrap();
    let local = &export["local_git_processes"];
    assert_eq!(local["source"], "local_git_process_observer");
    assert_eq!(local["scope"], "application_service");
    assert_eq!(local["coverage"], "local_git_subprocesses_only");
    assert_eq!(
        local["job_attribution"],
        "unknown_for_pre_job_and_shared_service_calls"
    );
    assert_eq!(local["remote_processes_observed"], false);
    assert!(
        local["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|record| record.get("argv").is_none()
                && record.get("path").is_none()
                && record.get("stdout").is_none())
    );
    assert_eq!(export["processes"]["complete"], false);
}
