use super::{evidence, fixture::Fixture, provider::Provider, script::Scenario};
use saya_agent::{AgentEvent, BudgetDimension, NoopEventSink};
use saya_types::{CodingEventKind, CodingFailureClass, JobState};
use std::time::Duration;

#[tokio::test]
async fn native_root_model_window_refuses_before_transport() {
    let provider = Provider::new(Scenario::Feedback {
        batches: 1,
        width: 4,
    });
    let fixture = Fixture::new(
        &provider.endpoint,
        4 * 1024 * 1024,
        // 6112 clears the composition check (4096 output + 1024 margin < 6112)
        // while the resulting request cap (992 tokens) still refuses the real
        // request at preflight, before any transport.
        Some(6_112),
        "child-context",
    )
    .await;
    let job = fixture.create().await;
    let result = tokio::time::timeout(Duration::from_secs(90), job.run(&NoopEventSink))
        .await
        .unwrap();
    let output = result.expect("preflight returns the canonical bounded-effort output");
    assert!(
        output.truncated,
        "output reserve leaves no admitted model input"
    );
    assert!(matches!(
        output.events.as_slice(),
        [AgentEvent::AnsweringBudgetStopped {
            dimension: BudgetDimension::RequestContextWindow
        }]
    ));
    assert_eq!(output.usage.input_tokens, 0);
    assert_eq!(output.usage.output_tokens, 0);
    assert!(output.answer.is_empty());
    assert!(output.tool_metadata.is_empty());
    assert_eq!(
        fixture
            .store
            .get_coding_job(job.job_id())
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Failed
    );
    let terminal = fixture
        .store
        .latest_coding_run_event(job.job_id())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(terminal.event(), CodingEventKind::RunTerminated {
        state: JobState::Failed, failure, ..
    } if failure.class == CodingFailureClass::BudgetStopped));
    assert!(
        fixture
            .store
            .list_coding_budget_reservations(job.job_id(), None, 100)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        provider.bodies().is_empty(),
        "no root transport or child effect before refusal"
    );
    assert!(
        fixture
            .store
            .list_coding_children(job.job_id(), None, 10)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        0
    );
    assert!(!fixture.repository.join("new.txt").exists());
}

#[tokio::test]
async fn native_child_uses_effective_model_preflight() {
    // Root's unknown model is byte-bounded; the child's actual endpoint model
    // has a known32Ki token table window. A globally tiny window would stop root
    // first and would never exercise this composition boundary.
    let provider = Provider::new(Scenario::Feedback {
        batches: 1,
        width: 1,
    });
    let fixture = Fixture::new(
        &provider.endpoint,
        4 * 1024 * 1024,
        None,
        "qwen2.5-coder:14b",
    )
    .await;
    let job = fixture.create().await;
    let output = tokio::time::timeout(Duration::from_secs(90), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    assert!(!output.truncated);
    let bodies = provider.bodies();
    let child = evidence::child_requests(&bodies);
    assert!(
        bodies
            .iter()
            .any(|body| body["model"] == "unknown-root-context-fixture")
    );
    assert!(
        !child.is_empty(),
        "real root delegation reached the child provider"
    );
    let input_cap = 32_768 - 4096 - 1024;
    for body in &child {
        assert_eq!(body["model"], "qwen2.5-coder:14b");
        // Content alone cannot exceed the complete neutral request ceiling.
        assert!(
            evidence::content_bytes(body) <= input_cap,
            "effective child model input ceiling bypassed: {} > {input_cap}",
            evidence::content_bytes(body)
        );
    }
    let results = evidence::results(&child);
    // The window, not the byte budget, is what bounds this child: the read it
    // asks for fits inside the 27 KiB effective input, so it comes back whole,
    // with the file's exact bytes and digest.
    let read: serde_json::Value =
        serde_json::from_str(&results["read-0-0"]).expect("the actual read is whole JSON");
    evidence::assert_valid_read_value(&read, &fixture, 0, "read-0-0");
    evidence::healthy(&fixture, &job, 2).await;
    evidence::reviewed(&fixture, &job).await;
}
