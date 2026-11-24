//! A job that ended because the provider failed resumes over the same in-place
//! workspace, under the same consent rules as a resumed paused job.

use super::{
    lease_probe::live_writer,
    provider::Provider,
    resume_support::{attempt, continuation_body, epoch, fixture, provider_failed, resume, run},
    script::{Fault, RESUMED, Scenario},
};
use saya_agent::CancellationToken;
use saya_cli::{CodingAuthorization, CodingError};

#[tokio::test]
async fn a_job_resume_after_a_provider_failure_continues_the_same_folder() {
    let provider = Provider::new(Scenario::Down {
        fault: Fault::Close,
    });
    let fixture = fixture(&provider).await;
    let job = attempt(&fixture, "resume-after-provider").await.unwrap();
    let error = run(&job)
        .await
        .expect_err("the provider failed on every attempt");
    assert!(error.to_string().contains("provider_error"), "{error}");
    assert!(
        provider.bodies().len() >= 2,
        "the run spent its retries before it ended"
    );
    let id = job.job_id().clone();
    assert_eq!(epoch(&fixture, &id).await, 1);
    // The live handle owns the in-place lease: the ended run is gone before a
    // new epoch takes it.
    drop(job);
    provider.recover();

    let approval =
        CodingAuthorization::scoped(&fixture.application.propose_resume(&id).await.unwrap())
            .unwrap();
    let resumed = fixture
        .application
        .resume(&id, &approval, CancellationToken::new())
        .await
        .expect("a job that ended on the provider failure resumes");
    assert_eq!(
        epoch(&fixture, &id).await,
        2,
        "the resume starts a new epoch"
    );
    // The resumed job holds the one-live-job lease over its folder: a live
    // write-scoped job over that folder is refused it.
    let busy = match live_writer(&fixture, "resume-after-provider-live").await {
        Err(error) => error,
        Ok(_) => panic!("one live job per folder"),
    };
    assert!(matches!(busy, CodingError::InPlaceRootBusy), "{busy:?}");

    run(&resumed).await.expect("the resumed run answers");
    let body = continuation_body(&provider);
    assert_eq!(body["version"], 1);
    assert_eq!(body["authority"], "none");
    assert!(
        body["budget"]["charged_model_requests"].as_u64().unwrap() >= 1,
        "the projection carries the earlier attempt's charges: {body}"
    );
    let status = fixture.application.status(&id).await.unwrap();
    assert_eq!(status["state"], "completed", "{status}");
    assert_eq!(epoch(&fixture, &id).await, 2, "the run kept its new epoch");

    // The lease goes with the handle: the folder admits a new writer.
    drop(resumed);
    drop(
        live_writer(&fixture, "resume-after-provider-next")
            .await
            .expect("the folder admits a new writer once the run's lease is released"),
    );
}

#[tokio::test]
async fn a_job_resume_after_a_provider_failure_answers_at_the_job_surface() {
    let provider = Provider::new(Scenario::Down {
        fault: Fault::Close,
    });
    let fixture = fixture(&provider).await;
    provider_failed(&fixture, "resume-surface").await;
    provider.recover();

    let result = resume(&fixture, "resume-surface", true)
        .await
        .expect("the job resumes over its own folder");
    assert_eq!(result["type"], "result", "{result}");
    assert_eq!(result["job_id"], "resume-surface", "{result}");
    assert_eq!(result["exit_code"], 0, "{result}");
    assert!(
        result["result"]
            .as_str()
            .unwrap_or_default()
            .contains(RESUMED),
        "{result}"
    );
}
