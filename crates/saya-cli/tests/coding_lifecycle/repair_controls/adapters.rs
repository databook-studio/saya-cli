use super::super::provider::Provider;
use super::support::{completed, request, resume};
use saya_agent::CancellationToken;
use saya_cli::{CodingCommand, CodingError, SlashCommand, dispatch_coding, parse_slash_command};
use saya_types::{BudgetDimension, JobState};
use std::fs;
#[tokio::test]
async fn oversized_brief_and_cancelled_owner_admit_no_new_invocation() {
    let provider = Provider::repair_controls();
    let (fixture, job, completion) =
        completed("repair-bounds", &provider, vec!["tracked.txt".into()]).await;
    let token = CancellationToken::new();
    let resumed = resume(&fixture, &job, token.clone()).await;
    for instruction in ["x".repeat(4097), "é".repeat(2049)] {
        assert!(matches!(
            resumed.follow_up(request(&completion, instruction)).await,
            Err(CodingError::InvalidRequest)
        ));
    }
    token.cancel();
    assert!(matches!(
        resumed
            .follow_up(request(&completion, "write for child-one; repair".into()))
            .await,
        Err(CodingError::InvalidRequest)
    ));
    assert_eq!(
        fixture
            .store
            .coding_child_invocation_history(job.job_id(), completion.result.agent_id())
            .await
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn cli_follow_up_preserves_redaction_lineage_and_completion() {
    let provider = Provider::repair_controls();
    let (fixture, job, completion) =
        completed("repair-cli", &provider, vec!["tracked.txt".into()]).await;
    let instruction =
        "write for child-one; repair the original path password=synthetic-repair-secret";
    let command = CodingCommand::FollowUp {
        id: job.job_id().as_str().into(),
        agent: completion.result.agent_id().as_str().into(),
        expected_completion_sequence: completion.sequence.get(),
        instruction: instruction.into(),
        approve: false,
        in_place: false,
        managed: false,
    };
    assert!(matches!(
        dispatch_coding(&fixture.application, command.clone()).await,
        Err(CodingError::ApprovalRequired)
    ));
    let tail = format!(
        "/follow-up {} {} --expected-completion-sequence {} '{instruction}' --approve",
        job.job_id(),
        completion.result.agent_id(),
        completion.sequence
    );
    let SlashCommand::Coding(approved) = parse_slash_command(&tail).unwrap().unwrap() else {
        panic!("shared coding grammar");
    };
    let output = dispatch_coding(&fixture.application, approved)
        .await
        .unwrap();
    assert_eq!(
        output["follow_up"]["previous_completion_sequence"],
        completion.sequence.get()
    );
    assert_ne!(
        output["follow_up"]["invocation_id"].as_str(),
        Some(completion.result.invocation_id().as_str())
    );
    assert_eq!(
        fixture
            .store
            .get_coding_job(job.job_id())
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Completed
    );
    let children = fixture
        .store
        .list_coding_children(job.job_id(), None, 100)
        .await
        .unwrap();
    assert_eq!(children.items.len(), 1);
    assert_eq!(children.items[0].state, JobState::Completed);
    let status = fixture.application.status(job.job_id()).await.unwrap();
    assert_eq!(
        status["children"][0]["invocation_lineage"]["follow_ups_prepared"],
        1
    );
    assert_eq!(status["completions"].as_array().unwrap().len(), 2);
    let records = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 100)
        .await
        .unwrap();
    // Helpers share the root working checkout W, so the follow-up's write
    // lands there; there is no per-child checkout to read it back from.
    let working = records
        .items
        .iter()
        .find(|record| {
            record
                .ownership
                .managed_checkout_id()
                .as_str()
                .starts_with("work-")
        })
        .expect("the root working checkout W exists");
    assert_eq!(
        fs::read(
            std::path::Path::new(working.ownership.managed_path().as_str()).join("tracked.txt")
        )
        .unwrap(),
        b"FIXED\n"
    );
    let export = fixture
        .application
        .export(job.job_id(), None, None)
        .await
        .unwrap()
        .to_string();
    assert!(!export.contains("synthetic-repair-secret"));
    assert!(
        !export.contains("repair the original path"),
        "new instruction is not a durable summary"
    );
    {
        let bodies = provider.bodies.lock().unwrap();
        assert!(
            bodies
                .iter()
                .any(|body| body.contains("password=[redacted]"))
        );
        assert!(
            bodies
                .iter()
                .all(|body| !body.contains("synthetic-repair-secret"))
        );
    }
    let reservations = fixture
        .store
        .list_coding_budget_reservations(job.job_id(), None, 100)
        .await
        .unwrap();
    assert_eq!(
        reservations
            .items
            .iter()
            .filter(
                |record| record.reservation.agent_id() == Some(completion.result.agent_id())
                    && record
                        .reservation
                        .charges()
                        .iter()
                        .any(|charge| charge.dimension() == BudgetDimension::ToolCalls)
            )
            .count(),
        // The child's first pass only reads (mode 21's child does not write,
        // so the child stays unmerged and this follow-up is allowed): one read
        // there, and the follow-up's read and write here.
        3
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
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
}
