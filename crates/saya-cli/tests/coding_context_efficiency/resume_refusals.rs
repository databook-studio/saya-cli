//! A resume is refused, with the outcome it ended on, for every job that did
//! not end on the provider failure: an agent failure keeps its own answer, a
//! missing approval keeps today's consent message, and a folder with a live job
//! is never given a second writer.

use super::{
    provider::Provider,
    resume_support::{attempt, epoch, fixture, provider_failed, resume, run},
    script::{Fault, Scenario},
};
use saya_cli::CodingError;

#[tokio::test]
async fn a_job_resume_refuses_a_job_that_ended_for_another_reason() {
    let provider = Provider::new(Scenario::MalformedCall);
    let fixture = fixture(&provider).await;
    let job = attempt(&fixture, "resume-refused").await.unwrap();
    let error = run(&job)
        .await
        .expect_err("the malformed call ends the run");
    assert!(error.to_string().contains("invalid_tool_call"), "{error}");
    drop(job);

    let refusal = resume(&fixture, "resume-refused", true)
        .await
        .expect_err("an agent failure is not resumable");
    let text = refusal.to_string();
    assert!(
        text.contains("job resume-refused finished (failed: invalid_tool_call)"),
        "{text}"
    );
    assert!(
        !text.contains("not owned") && !text.contains("no active runtime"),
        "the job is ours and it ended: {text}"
    );
}

#[tokio::test]
async fn a_job_resume_without_approval_is_refused_after_a_provider_failure() {
    let provider = Provider::new(Scenario::Down {
        fault: Fault::Close,
    });
    let fixture = fixture(&provider).await;
    let id = provider_failed(&fixture, "resume-consent").await;
    provider.recover();

    let refusal = resume(&fixture, id.as_str(), false)
        .await
        .expect_err("a resume needs fresh approval");
    assert!(
        matches!(refusal, CodingError::ApprovalRequired),
        "{refusal:?}"
    );
    assert_eq!(
        epoch(&fixture, &id).await,
        1,
        "no new epoch without consent"
    );
}

#[tokio::test]
async fn a_job_resume_refuses_a_provider_failure_while_the_folder_is_leased() {
    let provider = Provider::new(Scenario::Down {
        fault: Fault::Close,
    });
    let fixture = fixture(&provider).await;
    let id = provider_failed(&fixture, "resume-busy").await;
    provider.recover();
    // A live job over the same folder holds the one-live-job lease.
    let live = attempt(&fixture, "resume-busy-live").await.unwrap();

    let refusal = resume(&fixture, id.as_str(), true)
        .await
        .expect_err("the folder already has a live job");
    assert!(
        matches!(refusal, CodingError::InPlaceRootBusy),
        "{refusal:?}"
    );
    assert_eq!(
        epoch(&fixture, &id).await,
        1,
        "the refused resume took no epoch"
    );
    drop(live);
}
