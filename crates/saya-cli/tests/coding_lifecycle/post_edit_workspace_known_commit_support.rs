use super::control::{fail_and_join, root_checkout_intent};
use super::{Fixture, Provider, Run, extension_config, lsp_server};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use std::{fs, sync::Arc, time::Duration};
use tokio::time::Instant;

pub(super) async fn run_known_commit_append_failure() -> Run {
    let provider = Provider::post_edit_known_commit_failure();
    let profile = extension_config(&lsp_server(false, false, false), true);
    let fixture =
        Fixture::with_extensions("post-edit-record-failure", &provider.endpoint, &profile).await;
    let request = CodingJobRequest::new(
        JobId::parse("post-edit-record-failure").unwrap(),
        "Write the assigned file with helpers".into(),
        vec!["new.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let job_id = job.job_id().clone();
    let running = Arc::clone(&job);
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(180), running.run(&NoopEventSink))
            .await
            .expect("native job must settle")
    });
    let mut task = task;
    let (checkout_id, checkout_path) = match root_checkout_intent(
        &fixture.store,
        &job_id,
        Instant::now() + Duration::from_secs(60),
    )
    .await
    {
        Some(identity) => identity,
        None => {
            fail_and_join(
                &mut task,
                || provider.release(),
                || job.cancel(),
                "child checkout creation intent must identify the owned checkout",
            )
            .await
        }
    };
    let known_commit_path = fixture
        .root
        .join("coding")
        .join(job_id.as_str())
        .join(format!("known-commits-{checkout_id}.jsonl"));
    let record_parent = known_commit_path.parent().unwrap();
    if fs::create_dir_all(record_parent).is_err() || fs::create_dir(&known_commit_path).is_err() {
        fail_and_join(
            &mut task,
            || provider.release(),
            || job.cancel(),
            "inject a strict append-path conflict",
        )
        .await;
    }
    provider.release();
    let outcome = task.await.unwrap();

    Run {
        fixture,
        provider,
        job_id,
        checkout_id,
        outcome,
        checkout: checkout_path,
    }
}
