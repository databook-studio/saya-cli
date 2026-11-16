//! One admission function, one set of checks: every rule `delegate` and `helpers`
//! share is pinned here against the same `validate_delegate_tasks`. The shared
//! fixture and helpers live in `delegate_direct_tests/fixture.rs`.

#[cfg(unix)]
use super::CodingError;
#[cfg(unix)]
use super::CodingTask;
#[cfg(unix)]
use super::delegate_direct_fixture::{
    fixture as build, fixture_with_agents as build_with_agents, sized_task, task,
};
#[cfg(unix)]
use saya_types::AgentId;

#[cfg(unix)]
fn with_dependencies(agent_id: &str, dependencies: &[&str]) -> CodingTask {
    CodingTask {
        agent_id: AgentId::parse(agent_id).unwrap(),
        subagent_type: "general".into(),
        task: format!("write for {agent_id}"),
        write_paths: Vec::new(),
        dependencies: dependencies
            .iter()
            .map(|dependency| AgentId::parse(dependency).unwrap())
            .collect(),
    }
}

/// The 32 KiB cap is the only cap: 3000 bytes is admitted (the retired 2 KiB
/// cap refused it), 32768 is admitted and 32769 is refused.
#[cfg(unix)]
#[tokio::test]
async fn the_32_kib_cap_is_the_only_cap() {
    let fixture = build("one-cap", vec!["only.txt".into()]).await;
    let short = sized_task("child-a", 3000);
    let at_cap = sized_task("child-b", 32 * 1024);
    let over_cap = sized_task("child-c", 32 * 1024 + 1);
    assert!(
        fixture.job.validate_delegate_tasks(&[short]).is_ok(),
        "a 3000-byte assignment is inside the only cap"
    );
    assert!(
        fixture.job.validate_delegate_tasks(&[at_cap]).is_ok(),
        "a 32768-byte assignment is admitted"
    );
    assert!(
        matches!(
            fixture.job.validate_delegate_tasks(&[over_cap]),
            Err(CodingError::AssignmentTooLong)
        ),
        "a 32769-byte assignment is refused"
    );
}

/// The shared tree needs no recorded disjointness: a path an earlier recorded
/// batch owned does not refuse a later assignment.
#[cfg(unix)]
#[tokio::test]
async fn a_recorded_overlap_no_longer_refuses_a_task() {
    let fixture = build("recorded-overlap", vec!["only.txt".into()]).await;
    let recorded = serde_json::to_vec(&[task("child-a")]).unwrap();
    std::fs::write(fixture.job.records.join("tasks.jsonl"), recorded).unwrap();
    assert!(
        fixture
            .job
            .validate_delegate_tasks(&[task("child-b")])
            .is_ok(),
        "an overlapping recorded task is no longer a refusal"
    );
}

/// The id rules: a reserved root id, a blank assignment, a duplicate agent id
/// inside one batch, an id an earlier batch recorded and more than 8
/// dependencies all fail closed.
#[cfg(unix)]
#[tokio::test]
async fn the_id_rules_fail_closed() {
    let fixture = build("id-rules", vec!["only.txt".into()]).await;
    let mut blank = task("child-a");
    blank.task = "  \n ".into();
    for (label, tasks) in [
        ("a reserved root id", vec![task("coordinator")]),
        ("a blank assignment", vec![blank]),
        (
            "a duplicate agent id",
            vec![task("child-a"), task("child-a")],
        ),
        (
            "more than 8 dependencies",
            vec![with_dependencies(
                "child-a",
                &["d1", "d2", "d3", "d4", "d5", "d6", "d7", "d8", "d9"],
            )],
        ),
    ] {
        assert!(
            matches!(
                fixture.job.validate_delegate_tasks(&tasks),
                Err(CodingError::InvalidRequest)
            ),
            "{label} must be refused"
        );
    }
    let recorded = serde_json::to_vec(&[task("child-a")]).unwrap();
    std::fs::write(fixture.job.records.join("tasks.jsonl"), recorded).unwrap();
    assert!(
        matches!(
            fixture.job.validate_delegate_tasks(&[task("child-a")]),
            Err(CodingError::InvalidRequest)
        ),
        "an id an earlier batch recorded must be refused"
    );
}

/// The task-history bound counts what is already recorded: 255 + 1 is admitted,
/// 256 + 1 is refused.
#[cfg(unix)]
#[tokio::test]
async fn the_task_history_bound_counts_the_recorded_tasks() {
    let fixture = build("history-bound", vec!["only.txt".into()]).await;
    let path = fixture.job.records.join("tasks.jsonl");
    for recorded in [255, 256] {
        let batch: Vec<CodingTask> = (0..recorded)
            .map(|index| task(&format!("child-{index}")))
            .collect();
        std::fs::write(&path, serde_json::to_vec(&batch).unwrap()).unwrap();
        let extra = task("child-extra");
        assert_eq!(
            fixture.job.validate_delegate_tasks(&[extra]).is_ok(),
            recorded < 256,
            "{recorded} recorded + 1 must {}",
            if recorded < 256 {
                "be admitted"
            } else {
                "be refused"
            }
        );
    }
}

/// The batch bound `delegate` enforces before validation: one through 32 tasks.
#[cfg(unix)]
#[tokio::test]
async fn a_batch_is_one_through_32_tasks() {
    let fixture = build("batch-bound", vec!["only.txt".into()]).await;
    assert!(matches!(
        fixture.job.delegate(Vec::new()).await,
        Err(CodingError::InvalidRequest)
    ));
    let too_many: Vec<CodingTask> = (0..33)
        .map(|index| task(&format!("child-{index}")))
        .collect();
    assert!(matches!(
        fixture.job.delegate(too_many).await,
        Err(CodingError::InvalidRequest)
    ));
}

/// The helper type is one of the job's advertised ones: a name its registry
/// does not hold is refused by name, with the types it does, before any child
/// is registered or any history line written.
#[cfg(unix)]
#[tokio::test]
async fn a_subagent_type_outside_the_registry_is_refused_by_name() {
    let fixture = build("unknown-type", vec!["only.txt".into()]).await;
    let mut unknown = task("child-a");
    unknown.subagent_type = "reviewer".into();
    let refusal = fixture
        .job
        .validate_delegate_tasks(&[unknown])
        .expect_err("an unknown type is refused");
    assert!(
        matches!(
            &refusal,
            CodingError::UnknownSubagentType { requested, known }
                if requested == "reviewer" && known == "explore, general"
        ),
        "the refusal is typed and names the known types: {refusal:?}"
    );
    let message = refusal.to_string();
    for known in ["explore", "general"] {
        assert!(
            message.contains(known),
            "the refusal names every known type: {message}"
        );
    }
}

/// A configured type is admitted and composed: adding a helper type is
/// configuration, not a code change. The refused name is still refused in the
/// same job, so the registry is what admission reads.
#[cfg(unix)]
#[tokio::test]
async fn a_configured_type_is_admitted_and_composes() {
    let fixture = build_with_agents(
        "configured-type",
        "[agents.scout]\nrole = 'analyst'\n\n[agents.builder]\nrole = 'implementer'\n",
    )
    .await;
    let mut scout = task("child-scout");
    scout.subagent_type = "scout".into();
    let mut builder = task("child-builder");
    builder.subagent_type = "builder".into();
    assert!(
        fixture
            .job
            .validate_delegate_tasks(&[scout, builder])
            .is_ok(),
        "both configured types are advertised"
    );

    let mut unknown = task("child-unknown");
    unknown.subagent_type = "general-2".into();
    assert!(
        matches!(
            fixture.job.validate_delegate_tasks(&[unknown]),
            Err(CodingError::UnknownSubagentType { .. })
        ),
        "a type the registry does not hold is still refused"
    );

    // Composition reads the same registry: a configured type composes onto the
    // job's working checkout, and the one the registry does not hold does not.
    let mut scout = task("child-scout");
    scout.subagent_type = "scout".into();
    let mut registered = Vec::new();
    let staged = fixture
        .job
        .prepare_child(scout, &mut registered)
        .await
        .expect("a configured type composes");
    assert_eq!(staged.agent.as_str(), "child-scout");
}
