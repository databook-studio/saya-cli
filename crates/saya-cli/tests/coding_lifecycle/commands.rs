use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use serde_json::Value;

#[tokio::test]
async fn native_check_tool_retains_real_nonzero_exit_source_and_process_receipt() {
    let provider = Provider::checks();
    let fixture = Fixture::with_check("native-check", &provider.endpoint).await;
    let request = CodingJobRequest::new(
        JobId::parse("native-check").unwrap(),
        "Run native checks".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
        vec!["false".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    let receipts = receipts(&fixture, "native-check", "checks-");
    assert_eq!(
        receipts.len(),
        2,
        "both real native child dispatches must execute their checks"
    );
    for receipt in receipts {
        assert_eq!(receipt["successful"], false);
        assert_eq!(receipt["process"]["exit_code"], 1);
        assert_eq!(receipt["process"]["joined"], true);
        assert!(receipt["process"]["pid"].as_u64().unwrap() > 0);
        assert_eq!(
            receipt["receipt"]["source_sha256"].as_str().unwrap().len(),
            64
        );
    }
}
#[tokio::test]
async fn native_async_commands_settle_before_durable_child_completion() {
    let provider = Provider::commands();
    let fixture = Fixture::with_endpoint("native-command", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("native-command").unwrap(),
        "Run and settle native commands".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
        vec!["sleep".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    let receipts = receipts(&fixture, "native-command", "commands-");
    assert_eq!(receipts.len(), 2);
    for receipt in receipts {
        assert_eq!(receipt["program"], "sleep");
        assert_eq!(receipt["joined"], true);
        assert_eq!(receipt["cancelled"], true);
        assert_eq!(
            receipt["timed_out"], false,
            "invocation settlement runs before the 30 second process timeout"
        );
    }
    let completions = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap();
    assert_eq!(completions.items.len(), 2);
}
fn receipts(fixture: &Fixture, job: &str, prefix: &str) -> Vec<Value> {
    std::fs::read_dir(fixture.root.join("coding").join(job))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(prefix)
        })
        .flat_map(|path| {
            std::fs::read_to_string(path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect::<Vec<Value>>()
        })
        .collect()
}
