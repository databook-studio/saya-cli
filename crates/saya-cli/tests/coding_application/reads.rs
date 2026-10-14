use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, dispatch_coding,
};
use saya_types::JobId;

const JOB_ID_MESSAGE: &str = "job id must be 1-128 ASCII letters, digits, hyphens, or underscores";

async fn create_job(fixture: &Fixture, job: &str) -> JobId {
    let request = CodingJobRequest::new(
        JobId::parse(job).unwrap(),
        "inspect only".into(),
        vec![],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let runtime = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    runtime.job_id().clone()
}

fn owner_message(refusal: CodingError, message: &str) {
    let CodingError::Owner(source) = &refusal else {
        panic!("unexpected refusal variant: {refusal}");
    };
    assert_eq!(source.to_string(), message);
}

async fn list_page(fixture: &Fixture, after: Option<&str>) -> serde_json::Value {
    dispatch_coding(
        &fixture.application,
        CodingCommand::List {
            after: after.map(str::to_owned),
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn list_returns_each_created_job_with_its_state() {
    let fixture = Fixture::new("read-list-state").await;
    let first = create_job(&fixture, "job-list-a").await;
    let second = create_job(&fixture, "job-list-b").await;
    let listed = list_page(&fixture, None).await;
    assert_eq!(listed["next_cursor"], serde_json::Value::Null);
    assert_eq!(
        listed["jobs"],
        serde_json::json!([
            {"job_id": first.as_str(), "state": "running"},
            {"job_id": second.as_str(), "state": "running"},
        ])
    );
}

#[tokio::test]
async fn list_after_cursor_returns_only_later_jobs_and_an_empty_page_after_the_last() {
    let fixture = Fixture::new("read-list-after").await;
    let first = create_job(&fixture, "job-page-a").await;
    let second = create_job(&fixture, "job-page-b").await;
    let last = create_job(&fixture, "job-page-c").await;
    let listed = list_page(&fixture, Some(first.as_str())).await;
    assert_eq!(listed["next_cursor"], serde_json::Value::Null);
    assert_eq!(
        listed["jobs"],
        serde_json::json!([
            {"job_id": second.as_str(), "state": "running"},
            {"job_id": last.as_str(), "state": "running"},
        ])
    );
    let exhausted = list_page(&fixture, Some(last.as_str())).await;
    assert_eq!(exhausted["jobs"], serde_json::json!([]));
    assert_eq!(exhausted["next_cursor"], serde_json::Value::Null);
    let all = list_page(&fixture, None).await;
    assert_eq!(all["jobs"].as_array().unwrap().len(), 3);
    let Err(refusal) = dispatch_coding(
        &fixture.application,
        CodingCommand::List {
            after: Some("job page".into()),
        },
    )
    .await
    else {
        panic!("a malformed after cursor must be refused");
    };
    owner_message(refusal, JOB_ID_MESSAGE);
}

#[tokio::test]
async fn show_of_an_unknown_job_is_refused_until_the_job_exists() {
    let fixture = Fixture::new("read-show-unknown").await;
    let command = CodingCommand::Show {
        id: "job-show-unknown".into(),
    };
    assert!(matches!(
        dispatch_coding(&fixture.application, command.clone()).await,
        Err(CodingError::NotOwned)
    ));
    let job = create_job(&fixture, "job-show-unknown").await;
    let shown = dispatch_coding(&fixture.application, command)
        .await
        .unwrap();
    assert_eq!(shown["job_id"], job.as_str());
    assert_eq!(shown["state"], "running");
    assert_eq!(shown["children"], serde_json::json!([]));
    assert_eq!(shown["next_cursor"], serde_json::Value::Null);
}

#[tokio::test]
async fn show_of_a_malformed_job_id_is_refused_and_leaves_the_listing_unchanged() {
    let fixture = Fixture::new("read-show-malformed").await;
    let job = create_job(&fixture, "job-show-real").await;
    let Err(refusal) = dispatch_coding(
        &fixture.application,
        CodingCommand::Show {
            id: "job bad id".into(),
        },
    )
    .await
    else {
        panic!("a malformed job id must be refused");
    };
    owner_message(refusal, JOB_ID_MESSAGE);
    let listed = list_page(&fixture, None).await;
    assert_eq!(
        listed["jobs"],
        serde_json::json!([{"job_id": job.as_str(), "state": "running"}])
    );
    let shown = dispatch_coding(
        &fixture.application,
        CodingCommand::Show {
            id: job.as_str().into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(shown["job_id"], job.as_str());
}

#[tokio::test]
async fn events_after_zero_is_refused_and_a_cursor_past_the_last_sequence_is_empty() {
    let fixture = Fixture::new("read-events").await;
    let job = create_job(&fixture, "job-events").await;
    assert!(matches!(
        dispatch_coding(
            &fixture.application,
            CodingCommand::Events {
                id: job.as_str().into(),
                after: Some(0),
            },
        )
        .await,
        Err(CodingError::InvalidRequest)
    ));
    let page = dispatch_coding(
        &fixture.application,
        CodingCommand::Events {
            id: job.as_str().into(),
            after: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(page["next_cursor"], serde_json::Value::Null);
    // Create records the job's running state and its audited start approval
    // (request, then granted closure): three events, in order.
    let events = page["events"].as_array().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["sequence"], 1);
    assert_eq!(events[0]["job_id"], job.as_str());
    assert_eq!(events[1]["event"]["type"], "approval_requested");
    assert_eq!(events[2]["event"]["type"], "approval_closed");
    let past = dispatch_coding(
        &fixture.application,
        CodingCommand::Events {
            id: job.as_str().into(),
            after: Some(3),
        },
    )
    .await
    .unwrap();
    assert_eq!(past["events"], serde_json::json!([]));
    assert_eq!(past["next_cursor"], serde_json::Value::Null);
}
