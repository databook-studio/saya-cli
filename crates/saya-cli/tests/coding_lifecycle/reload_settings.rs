//! `/reload` swaps the session's runtime before the next input; coding
//! settings are read when a job is created, so the next job observes the
//! reloaded limits while a job that is already active keeps the limits it
//! was created with.

use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use std::sync::Arc;

fn request(id: &str) -> CodingJobRequest {
    CodingJobRequest::new(
        JobId::parse(id).unwrap(),
        "Inspect the repository".into(),
        vec!["a.txt".into()],
        Vec::new(),
    )
    .unwrap()
}

async fn create(application: &Arc<CodingApplication>, id: &str) -> Arc<saya_cli::NativeCodingJob> {
    let request = request(id);
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap()
}

/// A second application over the same repository and store, built from the
/// fixture's runtime after `change` altered its coding settings (what
/// `/reload` leaves); the first application keeps the runtime it opened with.
async fn reloaded_application(
    fixture: &Fixture,
    change: impl FnOnce(&mut saya_config::ResolvedCoding),
) -> Arc<CodingApplication> {
    let mut runtime = fixture.runtime.clone();
    change(&mut runtime.resolved.coding);
    CodingApplication::open(
        runtime,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap()
}

async fn recorded_child_slots(fixture: &Fixture, id: &str) -> u64 {
    fixture
        .store
        .get_coding_budget_policy(&JobId::parse(id).unwrap())
        .await
        .unwrap()
        .expect("a created job records its budget policy")
        .policy
        .child_slots()
}

#[tokio::test]
async fn next_job_observes_reloaded_limits_while_the_active_job_keeps_its_own() {
    let fixture = Fixture::new("reload-limits").await;
    let before = fixture.runtime.resolved.coding.max_children;
    assert!(before > 1, "the fixture needs room to lower the limit");
    let active = create(&fixture.application, "job-before-reload").await;
    assert_eq!(
        recorded_child_slots(&fixture, "job-before-reload").await,
        u64::try_from(before).unwrap()
    );

    let reloaded = reloaded_application(&fixture, |coding| coding.max_children = before - 1).await;

    let doctor = |application: Arc<CodingApplication>| async move {
        application.doctor(false).await.unwrap()["limits"]["max_children"].clone()
    };
    assert_eq!(
        doctor(reloaded.clone()).await,
        serde_json::json!(before - 1)
    );
    assert_eq!(
        doctor(fixture.application.clone()).await,
        serde_json::json!(before),
        "the application the active job runs under is not mutated"
    );

    let next = create(&reloaded, "job-after-reload").await;
    assert_eq!(
        recorded_child_slots(&fixture, "job-after-reload").await,
        u64::try_from(before - 1).unwrap(),
        "the next job is created under the reloaded settings"
    );
    assert_eq!(
        recorded_child_slots(&fixture, "job-before-reload").await,
        u64::try_from(before).unwrap(),
        "the active job keeps the limits it was created with"
    );
    assert!(!active.cancellation().is_cancelled());
    assert!(!next.cancellation().is_cancelled());
}
