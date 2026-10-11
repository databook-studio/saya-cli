use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{CodingAuthorization, CodingError, CodingJobRequest};
use saya_store::{NewCodingChild, ProjectPreferenceStore, SqliteStateStore};
use saya_types::{
    AgentId, CodingEventKind, CodingEventV1, CodingResultOutcome, InvocationId, JobId, JobState,
    JoinedResultV1, ProjectPreference, WorktreeId,
};
use std::num::NonZeroU64;

#[tokio::test]
async fn child_claim_requires_explicit_promotion() {
    let fixture = Fixture::new("promotion").await;
    let job_id = JobId::parse("job-promotion").unwrap();
    let request =
        CodingJobRequest::new(job_id.clone(), "observe only".into(), vec![], vec![]).unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let worktree = WorktreeId::from_canonical_path(&fixture.repository.to_string_lossy()).unwrap();
    assert!(
        stored_preferences(&fixture.store, &worktree)
            .await
            .is_empty()
    );
    let agent = AgentId::parse("promotion-child").unwrap();
    let invocation = InvocationId::parse("inv-promotion-child").unwrap();
    fixture
        .store
        .create_coding_child(NewCodingChild {
            job_id: job_id.clone(),
            agent_id: agent.clone(),
            current_invocation_id: invocation.clone(),
        })
        .await
        .unwrap();
    let answer = "always confirm the promotion flow with the user".to_string();
    let result = JoinedResultV1::new(
        job_id.clone(),
        agent.clone(),
        invocation.clone(),
        CodingResultOutcome::Completed,
        answer.clone(),
        false,
    )
    .unwrap();
    let events = fixture
        .store
        .list_coding_events(&job_id, None, 100)
        .await
        .unwrap();
    let sequence = NonZeroU64::new(
        events
            .events
            .last()
            .map(|event| event.sequence().get())
            .unwrap_or(0)
            + 1,
    )
    .unwrap();
    let event = CodingEventV1::new(
        sequence,
        job_id.clone(),
        Some(agent),
        Some(invocation),
        CodingEventKind::StateChanged {
            state: JobState::Completed,
        },
    )
    .unwrap();
    fixture
        .store
        .record_coding_completion(event, &result)
        .await
        .unwrap();
    let notes = fixture.application.notes(&job_id).await.unwrap();
    assert_eq!(notes["authority"], "none");
    assert_eq!(notes["findings"][0]["sequence"], sequence.get());
    assert_eq!(notes["findings"][0]["report"], answer.as_str());
    assert_eq!(notes["findings"][0]["outcome"], "completed");
    assert_eq!(notes["findings"][0]["accepted"], false);
    assert!(
        stored_preferences(&fixture.store, &worktree)
            .await
            .is_empty()
    );
    assert!(matches!(
        fixture
            .application
            .promote_finding(&job_id, sequence, &answer, false)
            .await,
        Err(CodingError::ApprovalRequired)
    ));
    assert!(
        stored_preferences(&fixture.store, &worktree)
            .await
            .is_empty()
    );
    assert!(matches!(
        fixture
            .application
            .promote_finding(&job_id, sequence, &answer, true)
            .await,
        Err(CodingError::ApprovalRequired)
    ));
    assert!(
        stored_preferences(&fixture.store, &worktree)
            .await
            .is_empty()
    );
    assert!(job.accept_finding(sequence).await.unwrap());
    let promoted = fixture
        .application
        .promote_finding(&job_id, sequence, &answer, true)
        .await
        .unwrap();
    assert_eq!(promoted["promoted"], true);
    assert_eq!(promoted["authority"], "none");
    let stored = stored_preferences(&fixture.store, &worktree).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].text, answer);
    assert_eq!(stored[0].worktree, worktree);
}

async fn stored_preferences(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
) -> Vec<ProjectPreference> {
    store.list_project_preferences(worktree).await.unwrap()
}
