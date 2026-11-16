use super::super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{ChildFollowUpRequest, CodingAuthorization, CodingJobRequest, NativeCodingJob};
use saya_store::CodingCompletion;
use saya_types::JobId;
use std::{sync::Arc, time::Duration};
/// Runs one direct-mode job whose scripted root delegates a single child and
/// joins it. The operator follow-up these tests exercise runs over the resumed
/// job against that child's own checkout, which the join's merge into the root
/// working checkout W leaves Active and untouched.
pub async fn completed(
    label: &str,
    provider: &Provider,
    paths: Vec<String>,
) -> (Fixture, Arc<NativeCodingJob>, CodingCompletion) {
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Prepare the owned file with helpers".into(),
        paths,
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    let completion = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();
    (fixture, job, completion)
}
pub async fn resume(
    fixture: &Fixture,
    job: &NativeCodingJob,
    token: CancellationToken,
) -> Arc<NativeCodingJob> {
    let approval = CodingAuthorization::scoped(
        &fixture
            .application
            .propose_resume(job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    fixture
        .application
        .resume(job.job_id(), &approval, token)
        .await
        .unwrap()
}
pub fn request(completion: &CodingCompletion, instruction: String) -> ChildFollowUpRequest {
    ChildFollowUpRequest {
        agent_id: completion.result.agent_id().clone(),
        expected_completion_sequence: completion.sequence,
        instruction,
    }
}
