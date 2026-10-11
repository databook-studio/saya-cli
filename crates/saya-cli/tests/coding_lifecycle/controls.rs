#[path = "control_support.rs"]
mod control_support;

use super::support::Fixture;
use control_support::ControlProvider;
use saya_agent::CancellationToken;
use saya_cli::{CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, CodingTask};
use saya_types::{AgentId, JobId};
use std::time::Duration;

#[tokio::test]
async fn live_controls_acknowledge_only_at_the_safe_boundary() {
    let provider = ControlProvider::new();
    let fixture = Fixture::with_endpoint("live-controls", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("live-controls").unwrap(),
        "Run controlled native children".into(),
        vec!["a.txt".into(), "b.txt".into()],
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
    assert!(matches!(
        job.control_command(CodingCommand::Pause {
            id: "live-controls".into(),
            agent: "coordinator".into(),
        })
        .await,
        Err(CodingError::NotOwned)
    ));
    job.delegate(vec![task("child-a", "a.txt"), task("child-b", "b.txt")])
        .await
        .unwrap();
    provider.wait_for_calls(2).await;

    assert_eq!(
        job.control_command(CodingCommand::Message {
            id: "live-controls".into(),
            agent: "child-a".into(),
            text: "check the boundary".into(),
        })
        .await
        .unwrap()["queued"],
        true
    );
    assert!(matches!(
        job.control_command(CodingCommand::Pause {
            id: "wrong-job".into(),
            agent: "child-a".into(),
        })
        .await,
        Err(CodingError::NotOwned)
    ));
    assert!(matches!(
        job.control_command(CodingCommand::Pause {
            id: "live-controls".into(),
            agent: "missing-child".into(),
        })
        .await,
        Err(CodingError::NotOwned)
    ));
    let pause = job
        .control_command(CodingCommand::Pause {
            id: "live-controls".into(),
            agent: "child-a".into(),
        })
        .await
        .unwrap();
    let ticket = pause["pause_requested"].as_u64().unwrap();
    assert_eq!(pause["acknowledged"], false);
    let wait = job.wait_pause("live-controls", "child-a", ticket);
    tokio::pin!(wait);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut wait)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(5), &mut wait)
            .await
            .unwrap()
            .unwrap()
    );
    assert_eq!(
        job.control_command(CodingCommand::Continue {
            id: "live-controls".into(),
            agent: "child-a".into(),
        })
        .await
        .unwrap()["continue_requested"],
        true
    );
    assert_eq!(
        job.control_command(CodingCommand::CancelChild {
            id: "live-controls".into(),
            agent: "child-b".into(),
        })
        .await
        .unwrap()["cancel_requested"],
        true
    );
    let _ = job.join().await;
}

fn task(agent: &str, path: &str) -> CodingTask {
    CodingTask {
        agent_id: AgentId::parse(agent).unwrap(),
        task: format!("write for {agent}"),
        write_paths: vec![path.into()],
        dependencies: vec![],
    }
}
