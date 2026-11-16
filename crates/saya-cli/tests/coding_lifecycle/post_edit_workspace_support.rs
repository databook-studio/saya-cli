pub(super) use super::super::extensions::{extension_config, lsp_server};
use super::super::provider::Provider;
use super::super::support::Fixture;
use saya_agent::{AgentOutput, CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingError, CodingJobRequest};
use saya_types::JobId;
use serde_json::Value;
#[cfg(target_os = "macos")]
use std::fs;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

pub(super) struct Run {
    pub(super) fixture: Fixture,
    pub(super) provider: Provider,
    pub(super) job_id: JobId,
    pub(super) checkout_id: String,
    pub(super) outcome: Result<AgentOutput, CodingError>,
    pub(super) checkout: PathBuf,
}

pub(super) async fn run(
    label: &str,
    provider: Provider,
    mode: u8,
    selector: bool,
    server: &str,
) -> Run {
    let profile = extension_config(server, selector);
    let fixture = Fixture::with_extensions(label, &provider.endpoint, &profile).await;
    let paths = match mode {
        30 => vec!["tracked.txt".to_owned(), "new.txt".to_owned()],
        31 | 33 => vec!["new.txt".to_owned()],
        _ => vec!["new.txt".to_owned(), "followup.txt".to_owned()],
    };
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Apply the assigned workspace change with helpers".into(),
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
    let job_id = job.job_id().clone();
    let outcome = tokio::time::timeout(Duration::from_secs(180), job.run(&NoopEventSink))
        .await
        .expect("the native job must settle");
    // A helper shares the root's working checkout W: there is no
    // per-helper checkout, so the post-edit hook under test runs over W, whose
    // id keys the receipts. A prepared candidate (`candidate-…`) may also
    // exist, so select W by its derived `work-` id.
    let checkout = fixture
        .store
        .list_managed_checkouts(&job_id, None, 8)
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|record| {
            record
                .ownership
                .managed_checkout_id()
                .as_str()
                .starts_with("work-")
        })
        .expect("the root working checkout W exists");
    Run {
        fixture,
        provider,
        job_id,
        checkout_id: checkout.ownership.managed_checkout_id().as_str().to_owned(),
        outcome,
        checkout: PathBuf::from(checkout.ownership.managed_path().as_str()),
    }
}

pub(super) fn tool_results(provider: &Provider) -> BTreeMap<String, Value> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|body| {
            let request: Value = serde_json::from_str(body).unwrap();
            request["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|message| {
                    let id = message["tool_call_id"].as_str()?;
                    serde_json::from_str(message["content"].as_str()?)
                        .ok()
                        .map(|value| (id.to_owned(), value))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(super) fn record_path(run: &Run, prefix: &str) -> PathBuf {
    run.fixture
        .root
        .join("coding")
        .join(run.job_id.as_str())
        .join(format!("{prefix}{}.jsonl", run.checkout_id))
}

#[cfg(target_os = "macos")]
pub(super) fn jsonl(path: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[cfg(target_os = "macos")]
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
