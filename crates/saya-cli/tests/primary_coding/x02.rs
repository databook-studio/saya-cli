use super::support::Fixture;
#[path = "x02_support.rs"]
mod x02_support;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use std::path::PathBuf;

#[tokio::test]
async fn coding_mcp_dispatches_only_an_explicit_read_only_profile() {
    let run = invoke(&x02_support::profile(
        &x02_support::server(x02_support::listed("inspect"), x02_support::result()),
        Some("read_only"),
    ))
    .await;

    assert!(run.checkout.join("mcp-called").exists());
    assert!(run.provider.saw_result());
    assert!(run.records.contains("\"phase\":\"spawned\""));
    assert!(run.records.contains("\"phase\":\"settled\""));
    assert!(run.records.contains("\"joined\":true"));
    assert!(!run.records.contains("safe MCP result"));
}

#[tokio::test]
async fn coding_mcp_refuses_untrusted_effects_before_server_side_effects() {
    for effect in [None, Some("unknown"), Some("mutating")] {
        let script = x02_support::server(x02_support::listed("inspect"), x02_support::result());
        let profile = effect.map_or_else(String::new, |effect| {
            x02_support::profile(&script, Some(effect))
        });
        let run = invoke(&profile).await;

        assert!(!run.checkout.join("mcp-called").exists(), "{effect:?}");
        assert!(!run.records.contains("safe MCP result"));
    }
}

#[tokio::test]
async fn coding_mcp_rejects_untrusted_listings_and_wrong_responses_before_calling() {
    for listing in [
        x02_support::listed("other"),
        x02_support::invalid_schema(),
        x02_support::too_many_tools(),
        x02_support::paged(),
        x02_support::wrong_id(),
        x02_support::malformed(),
    ] {
        let run = invoke(&x02_support::profile(
            &x02_support::server(listing, x02_support::result()),
            Some("read_only"),
        ))
        .await;

        assert!(!run.checkout.join("mcp-called").exists());
        assert!(!run.records.contains("safe MCP result"));
    }
}

#[tokio::test]
async fn coding_mcp_drops_an_oversized_server_result_after_the_authorized_call() {
    let run = invoke(&x02_support::profile(
        &x02_support::server_with_oversized_result(x02_support::listed("inspect")),
        Some("read_only"),
    ))
    .await;

    assert!(run.checkout.join("mcp-called").exists());
    assert!(!run.provider.saw_result());
    assert!(!run.records.contains("oversized MCP result"));
}

#[tokio::test]
async fn coding_mcp_rejects_an_oversized_schema_before_calling_the_tool() {
    let run = invoke(&x02_support::profile(
        &x02_support::server_with_oversized_schema(),
        Some("read_only"),
    ))
    .await;

    assert!(!run.checkout.join("mcp-called").exists());
    assert!(!run.records.contains("oversized schema"));
}

async fn invoke(profile: &str) -> Invocation {
    let provider = x02_support::Provider::new();
    let fixture = Fixture::with_extensions("x02-mcp", &provider.endpoint, profile).await;
    let request = CodingJobRequest::new(
        JobId::parse("x02-mcp").unwrap(),
        "Call the configured MCP inspection tool".into(),
        vec![],
        vec![x02_support::PROGRAM.into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(20), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    let checkout = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 4)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();
    let records = std::fs::read_dir(fixture.root.join("coding").join(job.job_id().as_str()))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("extensions-")
        })
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect();
    Invocation {
        _fixture: fixture,
        provider,
        checkout: PathBuf::from(checkout.ownership.managed_path().as_str()),
        records,
    }
}

struct Invocation {
    _fixture: Fixture,
    provider: x02_support::Provider,
    checkout: PathBuf,
    records: String,
}
