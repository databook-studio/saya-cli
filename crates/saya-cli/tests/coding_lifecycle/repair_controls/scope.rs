use super::super::provider::Provider;
use super::support::{completed, request, resume};
use saya_agent::CancellationToken;
use saya_cli::CodingError;
use serde_json::Value;
use std::{fs, time::Duration};
#[tokio::test]
async fn changed_task_cannot_expand_original_child_ownership() {
    let provider = Provider::repair_controls();
    let (fixture, job, completion) = completed(
        "repair-task-scope",
        &provider,
        vec!["tracked.txt".into(), "extra.txt".into()],
    )
    .await;
    let path = fixture
        .root
        .join("coding")
        .join(job.job_id().as_str())
        .join("tasks.jsonl");
    let mut tasks: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    tasks[0]["write_paths"] = serde_json::json!(["tracked.txt", "extra.txt"]);
    let mut bytes = serde_json::to_vec(&tasks).unwrap();
    bytes.push(b'\n');
    fs::write(&path, bytes).unwrap();
    let resumed = resume(&fixture, &job, CancellationToken::new()).await;
    let result = resumed
        .follow_up(request(
            &completion,
            "write for child-one; repair only original ownership".into(),
        ))
        .await;
    if result.is_ok() {
        tokio::time::timeout(Duration::from_secs(60), resumed.join())
            .await
            .unwrap()
            .unwrap();
    }
    assert!(
        matches!(result, Err(CodingError::NotOwned)),
        "task history cannot widen bound scope: {result:?}"
    );
    assert_eq!(
        fixture
            .store
            .coding_child_invocation_history(job.job_id(), completion.result.agent_id())
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(!fixture.repository.join("extra.txt").exists());
}
