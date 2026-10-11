use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{
    CodingApplication, CodingAuthorization, CodingJobRequest, GlobalOptions, load_with_sources,
};
use saya_types::JobId;
use std::collections::BTreeMap;

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
        .export(request.job_id(), None, None)
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
    let prior_identity = local["service_identity"].clone();
    let prior_sequence = local["records"][0]["sequence"].clone();
    let runtime = load_with_sources(
        &GlobalOptions::default(),
        &fixture.repository,
        &fixture.root,
        BTreeMap::new(),
    )
    .unwrap();
    let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let reopened = CodingApplication::open(
        runtime,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &git,
        std::path::Path::new("/usr/bin"),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let reopened_export = reopened
        .export(request.job_id(), None, None)
        .await
        .unwrap();
    let reopened_local = &reopened_export["local_git_processes"];
    assert_ne!(reopened_local["service_identity"], prior_identity);
    assert!(
        reopened_local["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| {
                record["service_identity"] == prior_identity && record["sequence"] == prior_sequence
            })
    );
    assert_eq!(reopened_export["processes"]["complete"], false);
}
