//! A run with no database: the database text never reaches its request.
//!
//! The database part of the prompt is the database plugin's section (DB1), so a
//! run that activates no database plugin — no profile resolved, no registry
//! supplied — must send none of those words. The capture is the scripted
//! provider's own first request body for a `-p`/job root, the surface a
//! database-less run reaches.

use std::time::Duration;

use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;

use super::{fixture::Fixture, provider::Provider, script::Scenario};

const JOB_ID: &str = "parity-no-database";
const TASK: &str = "summarise the tracked notes";
const RUN_CEILING: Duration = Duration::from_secs(120);

/// The words the database prompt's texts contribute. None may reach a request
/// from a run whose database plugin never activated.
pub(super) const DATABASE_WORDS: [&str; 3] = ["Database", "dialect", "schema"];

/// The job root's first provider request body, from a run with no database
/// profile and no plugin registry.
pub(super) fn first_request_body() -> String {
    let provider = Provider::new(Scenario::Feedback {
        batches: 0,
        width: 0,
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let fixture = Fixture::new(&provider.endpoint, 256 * 1024, Some(8_000_000), "parity").await;
        let request = CodingJobRequest::new(
            JobId::parse(JOB_ID).unwrap(),
            TASK.into(),
            vec!["notes.txt".into()],
        )
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
        provider
            .raw_bodies()
            .into_iter()
            .next()
            .expect("the job root sent a first provider request")
    })
}
