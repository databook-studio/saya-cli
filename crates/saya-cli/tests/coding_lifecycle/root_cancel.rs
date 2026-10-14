use super::controls::control_support::ControlProvider;
use super::support::Fixture;
use saya_agent::{CancellationReason, CancellationToken};
use saya_cli::{CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, CodingTask};
use saya_harness::coding::coordinator::{ChildRunError, ScheduleChildError, ScheduleStop};
use saya_types::{AgentId, CodingResultOutcome, JobId};
use std::time::Duration;

#[tokio::test]
async fn root_cancel_stops_every_child_and_is_separate_from_selecting_one() {
    let provider = ControlProvider::new();
    let fixture = Fixture::with_endpoint("root-cancel", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("root-cancel").unwrap(),
        "Run two held children for the root cancel leg".into(),
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
    job.delegate(vec![task("child-a", "a.txt"), task("child-b", "b.txt")])
        .await
        .unwrap();
    provider.wait_for_calls(2).await;
    assert!(!job.cancellation().is_cancelled());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);

    for agent in ["child-a", "child-b"] {
        let pause = job
            .control_command(CodingCommand::Pause {
                id: "root-cancel".into(),
                agent: agent.into(),
            })
            .await
            .unwrap();
        let ticket = pause["pause_requested"].as_u64().unwrap();
        assert!(
            tokio::time::timeout_at(deadline, job.wait_pause("root-cancel", agent, ticket))
                .await
                .unwrap()
                .unwrap(),
            "{agent} must acknowledge its matching pause at a safe boundary"
        );
    }

    assert_eq!(
        job.control_command(CodingCommand::CancelChild {
            id: "root-cancel".into(),
            agent: "child-a".into(),
        })
        .await
        .unwrap()["cancel_requested"],
        true
    );
    assert!(
        !job.cancellation().is_cancelled(),
        "selecting one child must not cancel the job root"
    );
    loop {
        let status = job.status().await.unwrap();
        if child_state(&status, "child-a") == "cancelled" {
            assert_eq!(
                child_state(&status, "child-b"),
                "running",
                "the untouched child must still be live after one child was selected"
            );
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "child-a never settled as cancelled: {status}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    job.cancel();
    assert!(job.cancellation().is_cancelled());
    let report = job.join().await.unwrap();
    assert_eq!(
        report.stop,
        Some(ScheduleStop::Cancelled(CancellationReason::User))
    );
    assert!(!report.effects_uncertain);
    assert!(report.not_started.is_empty());
    assert_eq!(report.outcomes.len(), 2);
    for outcome in &report.outcomes {
        assert!(matches!(
            &outcome.result,
            Err(ScheduleChildError::Run(ChildRunError::Cancelled {
                reason: CancellationReason::User
            }))
        ));
        assert!(outcome.completion_sequence.is_some());
    }

    let status = job.status().await.unwrap();
    for agent in ["child-a", "child-b"] {
        assert_eq!(child_state(&status, agent), "cancelled");
    }
    let completions = fixture
        .store
        .list_coding_completions(job.job_id(), None, 32)
        .await
        .unwrap();
    assert_eq!(completions.items.len(), 2);
    assert!(
        completions
            .items
            .iter()
            .all(|item| item.result.outcome() == CodingResultOutcome::Cancelled)
    );
    assert!(matches!(
        job.control_command(CodingCommand::CancelChild {
            id: "root-cancel".into(),
            agent: "child-b".into(),
        })
        .await,
        Err(CodingError::NotOwned)
    ));
}

fn child_state<'a>(status: &'a serde_json::Value, agent: &str) -> &'a str {
    status["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|child| child["agent_id"] == agent)
        .unwrap()["state"]
        .as_str()
        .unwrap()
}

fn task(agent: &str, path: &str) -> CodingTask {
    CodingTask {
        agent_id: AgentId::parse(agent).unwrap(),
        task: format!("write for {agent}"),
        write_paths: vec![path.into()],
        dependencies: vec![],
    }
}
