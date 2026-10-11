use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_store::ProjectPreferenceStore;
use saya_types::{JobId, WorktreeId};
use std::sync::atomic::Ordering;

const MARKER: &str = "INJECTED ROOT AUTHORITY: scoped project preference marker";

async fn create_and_run(label: &str, preference_for_original: bool) -> bool {
    let provider = Provider::new(false);
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let original = worktree_id(&fixture.repository);
    let other_path = fixture.root.join("other-worktree");
    std::fs::create_dir(&other_path).unwrap();
    let other = worktree_id(&other_path);
    let selected = if preference_for_original {
        &original
    } else {
        &other
    };
    fixture
        .store
        .add_project_preference(selected, MARKER, 1)
        .await
        .unwrap();

    let request = request(label);
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let output = tokio::time::timeout(std::time::Duration::from_secs(20), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    assert!(!output.truncated);
    provider.saw_injection.load(Ordering::Acquire)
}

fn request(id: &str) -> CodingJobRequest {
    CodingJobRequest::new(
        JobId::parse(id).unwrap(),
        "Make two native child changes".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
        vec![],
    )
    .unwrap()
}

fn worktree_id(path: &std::path::Path) -> WorktreeId {
    let canonical = path.canonicalize().unwrap();
    WorktreeId::from_canonical_path(canonical.to_str().unwrap()).unwrap()
}

#[tokio::test]
async fn native_create_uses_only_preferences_for_the_original_worktree() {
    assert!(create_and_run("project-pref-original", true).await);
    assert!(!create_and_run("project-pref-other", false).await);
}

#[tokio::test]
async fn fresh_approved_resume_captures_current_original_project_preferences() {
    let provider = Provider::new(false);
    let fixture = Fixture::with_endpoint("project-pref-resume", Some(&provider.endpoint)).await;
    let request = request("project-pref-resume");
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();

    fixture
        .store
        .add_project_preference(&worktree_id(&fixture.repository), MARKER, 1)
        .await
        .unwrap();
    let approval = CodingAuthorization::scoped(
        &fixture
            .application
            .propose_resume(job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    let resumed = fixture
        .application
        .resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        resumed.run(&NoopEventSink),
    )
    .await
    .unwrap()
    .unwrap();

    assert!(!output.truncated);
    assert!(provider.saw_injection.load(Ordering::Acquire));
}
