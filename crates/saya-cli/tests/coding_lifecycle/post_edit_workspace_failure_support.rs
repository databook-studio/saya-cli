use super::super::super::{provider::Provider, support::Fixture};
use super::super::support::Run;
#[cfg(target_os = "macos")]
use super::super::support::tool_results;
pub(super) use super::super::support::{extension_config, lsp_server};
#[cfg(target_os = "macos")]
use saya_agent::{CancellationToken, NoopEventSink};
#[cfg(target_os = "macos")]
use saya_cli::{CodingAuthorization, CodingJobRequest};
#[cfg(target_os = "macos")]
use saya_types::JobId;
#[cfg(target_os = "macos")]
use std::{fs, sync::Arc, time::Duration};
#[cfg(target_os = "macos")]
use tokio::time::Instant;
#[path = "post_edit_workspace_failure_control.rs"]
mod control;
#[cfg(target_os = "macos")]
use control::{fail_and_join, root_checkout_intent};
#[path = "post_edit_workspace_known_commit_support.rs"]
mod known_commit;

pub(super) async fn run_known_commit_append_failure() -> Run {
    known_commit::run_known_commit_append_failure().await
}

#[cfg(target_os = "macos")]
pub(super) async fn run_stale_after_exchange() -> Run {
    let provider = Provider::post_edit_stale();
    let profile = extension_config(&lsp_server(true, false, false), true);
    let fixture = Fixture::with_extensions("post-edit-stale", &provider.endpoint, &profile).await;
    let request = CodingJobRequest::new(
        JobId::parse("post-edit-stale").unwrap(),
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
                || {},
                || job.cancel(),
                "child checkout creation intent must identify the owned checkout",
            )
            .await
        }
    };
    let ready_deadline = Instant::now() + Duration::from_secs(60);
    let ready = checkout_path.join(".post-edit-stale-ready");
    loop {
        if ready.is_file() {
            break;
        }
        if Instant::now() >= ready_deadline {
            let release = checkout_path.join(".post-edit-stale-release");
            fail_and_join(
                &mut task,
                || {
                    let _ = fs::write(release, b"release");
                },
                || job.cancel(),
                "LSP must open the committed file",
            )
            .await;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    if fs::write(checkout_path.join("new.txt"), b"external mutation\n").is_err() {
        let release = checkout_path.join(".post-edit-stale-release");
        fail_and_join(
            &mut task,
            || {
                let _ = fs::write(release, b"release");
            },
            || job.cancel(),
            "the independent owner changes the file during the exchange",
        )
        .await;
    }
    if fs::write(checkout_path.join(".post-edit-stale-release"), b"release").is_err() {
        fail_and_join(
            &mut task,
            || {},
            || job.cancel(),
            "release the real confined LSP fixture",
        )
        .await;
    }
    let refusal_deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let results = tool_results(&provider);
        let stale = results
            .get("call-write")
            .is_some_and(|result| result["post_edit_diagnostics"]["status"] == "stale");
        let refused = results
            .get("call-edit")
            .and_then(|result| result["error"].as_str())
            .is_some_and(|error| error.contains("expected_digest"));
        if stale && refused {
            break;
        }
        if Instant::now() >= refusal_deadline {
            fail_and_join(
                &mut task,
                || provider.release(),
                || job.cancel(),
                "stale feedback and its omitted-digest refusal must reach the child model",
            )
            .await;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // The stale observation and refusal are captured; restore the test's
    // external mutation before allowing the child to finish so final checkout
    // ownership is not conflated with the witness contract under test.
    if !fs::read(checkout_path.join("new.txt"))
        .is_ok_and(|bytes| bytes.as_slice() == b"external mutation\n")
    {
        fail_and_join(
            &mut task,
            || provider.release(),
            || job.cancel(),
            "the stale exchange must observe the external bytes before restoration",
        )
        .await;
    }
    if fs::write(checkout_path.join("new.txt"), b"initial\n").is_err() {
        fail_and_join(
            &mut task,
            || provider.release(),
            || job.cancel(),
            "restore the fixture's committed bytes after observing stale refusal",
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

#[cfg(target_os = "macos")]
pub(super) async fn run_observer_dual_failure() -> Run {
    let provider = Provider::post_edit_observer_failure();
    let profile = extension_config(&lsp_server(false, true, true), true);
    let fixture =
        Fixture::with_extensions("post-edit-observer-failure", &provider.endpoint, &profile).await;
    let request = CodingJobRequest::new(
        JobId::parse("post-edit-observer-failure").unwrap(),
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
                || {},
                || job.cancel(),
                "child checkout creation intent must identify the owned checkout",
            )
            .await
        }
    };
    let spawn_deadline = Instant::now() + Duration::from_secs(60);
    let extension_path = fixture
        .root
        .join("coding")
        .join(job_id.as_str())
        .join(format!("extensions-{checkout_id}.jsonl"));
    loop {
        let started = fs::read_to_string(&extension_path)
            .is_ok_and(|records| records.contains("\"phase\":\"spawned\""));
        if started {
            break;
        }
        if Instant::now() >= spawn_deadline {
            let release = checkout_path.join(".post-edit-release");
            fail_and_join(
                &mut task,
                || {
                    let _ = fs::write(release, b"release");
                },
                || job.cancel(),
                "the confined LSP must spawn",
            )
            .await;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut records = match fs::read(&extension_path) {
        Ok(records) => records,
        Err(_) => {
            let release = checkout_path.join(".post-edit-release");
            fail_and_join(
                &mut task,
                || {
                    let _ = fs::write(release, b"release");
                },
                || job.cancel(),
                "spawn observer record exists",
            )
            .await
        }
    };
    const MAX_EXTENSION_RECORD_BYTES: usize = 1024 * 1024;
    let prefix = b"{\"fixture_padding\":\"";
    let suffix = b"\"}\n";
    if records.len() >= MAX_EXTENSION_RECORD_BYTES {
        let release = checkout_path.join(".post-edit-release");
        fail_and_join(
            &mut task,
            || {
                let _ = fs::write(release, b"release");
            },
            || job.cancel(),
            "spawn observer record stays below its append bound",
        )
        .await;
    }
    let padding = MAX_EXTENSION_RECORD_BYTES - records.len();
    if padding <= prefix.len() + suffix.len() {
        let release = checkout_path.join(".post-edit-release");
        fail_and_join(
            &mut task,
            || {
                let _ = fs::write(release, b"release");
            },
            || job.cancel(),
            "observer record has room for the bounded padding record",
        )
        .await;
    }
    records.extend_from_slice(prefix);
    records.extend(std::iter::repeat_n(
        b'x',
        padding - prefix.len() - suffix.len(),
    ));
    records.extend_from_slice(suffix);
    if records.len() != MAX_EXTENSION_RECORD_BYTES {
        let release = checkout_path.join(".post-edit-release");
        fail_and_join(
            &mut task,
            || {
                let _ = fs::write(release, b"release");
            },
            || job.cancel(),
            "padding reaches the actual observer append bound",
        )
        .await;
    }
    if fs::write(&extension_path, records).is_err() {
        let release = checkout_path.join(".post-edit-release");
        fail_and_join(
            &mut task,
            || {
                let _ = fs::write(release, b"release");
            },
            || job.cancel(),
            "fill the actual observer append bound",
        )
        .await;
    }
    if fs::write(checkout_path.join(".post-edit-release"), b"release").is_err() {
        fail_and_join(
            &mut task,
            || {},
            || job.cancel(),
            "release the real confined LSP fixture",
        )
        .await;
    }
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
