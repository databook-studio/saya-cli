//! The `-p` / job-root surface and its helper, driven in process by the
//! shared context-efficiency fixture. One scripted run: the root calls
//! `helpers` on its first turn, and the helper makes its own first request.
//! The root's first body is the job surface; the first body whose task block
//! names a child is the helper surface.
//!
//! The task states a URL and a helper so the evidence-gated web and helper
//! tools are advertised, matching the surface as a user reaches it.
//!
//! The script answers each surface on its second turn, so both surfaces make
//! two requests and the request baseline can compare the two prefixes.

use std::{path::PathBuf, time::Duration};

use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;

use super::{
    capture::Advertised,
    fixture::Fixture,
    provider::{Provider, is_child},
    script::Scenario,
};

const JOB_ID: &str = "run-core-parity";
const TASK: &str = "write for child-context; inspect actual files with a helper at \
                    https://example.invalid";
const RUN_CEILING: Duration = Duration::from_secs(120);

pub(super) struct Captured {
    pub(super) job: Advertised,
    pub(super) helper: Advertised,
}

/// The run's request bodies, split by surface, in arrival order; plus the
/// fixture roots those bodies may name.
pub(super) struct Conversation {
    pub(super) jobs: Vec<String>,
    pub(super) helpers: Vec<String>,
    pub(super) roots: Vec<PathBuf>,
}

pub(super) fn capture() -> Captured {
    let conversation = converse();
    let first = |bodies: &[String], surface: &str| {
        let body = bodies
            .first()
            .unwrap_or_else(|| panic!("the {surface} surface sent no provider request"));
        let value: serde_json::Value =
            serde_json::from_str(body).expect("the provider request body is JSON");
        Advertised::from_body(&value)
    };
    Captured {
        job: first(&conversation.jobs, "job root"),
        helper: first(&conversation.helpers, "helper"),
    }
}

pub(super) fn converse() -> Conversation {
    let provider = Provider::new(Scenario::Feedback {
        batches: 0,
        width: 0,
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let fixture = Fixture::with_sqlite_profile(
            &provider.endpoint,
            256 * 1024,
            Some(8_000_000),
            "parity-helper",
            "local",
        )
        .await;
        let request = CodingJobRequest::new(
            JobId::parse(JOB_ID).unwrap(),
            TASK.into(),
            vec!["new.txt".into()],
        )
        .unwrap()
        .with_database_profiles(vec!["local".into()])
        .unwrap();
        let approval =
            CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap())
                .unwrap();
        let job = fixture
            .application
            .create(request, &approval, CancellationToken::new())
            .await
            .unwrap();
        tokio::time::timeout(RUN_CEILING, job.run(&NoopEventSink))
            .await
            .expect("the scripted job terminates")
            .unwrap();
        let mut jobs = Vec::new();
        let mut helpers = Vec::new();
        for body in provider.raw_bodies() {
            let value: serde_json::Value =
                serde_json::from_str(&body).expect("the provider request body is JSON");
            if is_child(&value) {
                helpers.push(body);
            } else {
                jobs.push(body);
            }
        }
        Conversation {
            jobs,
            helpers,
            roots: vec![fixture.root.clone()],
        }
    })
}
