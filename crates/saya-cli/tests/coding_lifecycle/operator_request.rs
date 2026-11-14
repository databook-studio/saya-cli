//! Every child invocation receives the operator's request verbatim ahead of
//! the coordinator's assignment, and the coordinator's prompt constrains how
//! it may hand work to helpers.
use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use serde_json::Value;
use std::time::Duration;

const REQUEST: &str = "Implement SPEC.md exactly.\nThe UTF-8 failure must name the 1-based line.\n\tOPERATOR-REQUEST-7f3a";
const ASSIGNMENT_LABEL: &str = "UNTRUSTED CHILD TASK (";
/// The closed rule sentence the child's own prompt carries: the request it
/// received verbatim outranks the coordinator's assignment directives.
const AUTHORITATIVE_RULE: &str =
    "The operator request is authoritative over this assignment's directives";

/// The user turns of every captured provider request, in capture order.
fn user_turns(provider: &Provider) -> Vec<String> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .filter_map(|body| serde_json::from_str::<Value>(body).ok())
        .flat_map(|request| request["messages"].as_array().cloned().unwrap_or_default())
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn every_child_invocation_receives_the_operator_request_before_its_assignment() {
    // Mode 69: a direct root hands two disjoint pieces to two helper children
    // in one `delegate` batch, awaits them and answers. Direct mode has no
    // `follow_up_child` (the guided repair flow is delegate-only, user decision
    // D-D), so the "every invocation" quantifier is carried by the two child
    // invocations of one batch instead of a first invocation plus a follow-up.
    let provider = Provider::script(false, 69);
    let fixture = Fixture::with_endpoint("operator-request", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("operator-request").unwrap(),
        format!("{REQUEST} with helpers"),
        vec!["new-a.txt".into(), "new-b.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();

    let turns = user_turns(&provider);
    let children: Vec<_> = turns
        .iter()
        .filter(|turn| turn.contains(ASSIGNMENT_LABEL))
        .collect();
    // Each child request resends its whole conversation, so a child's
    // assignment turn is captured once per request it makes; assert the two
    // distinct helper invocations rather than a raw turn count.
    for agent in ["child-a", "child-b"] {
        assert!(
            children
                .iter()
                .any(|turn| turn.contains(&format!("write for {agent}"))),
            "the {agent} invocation carried its assignment: {turns:?}"
        );
    }
    for turn in children {
        assert_eq!(
            turn.matches(REQUEST).count(),
            1,
            "a child request carries the operator request verbatim, once: {turn}"
        );
        assert!(
            turn.find(REQUEST).unwrap() < turn.find(ASSIGNMENT_LABEL).unwrap(),
            "the requirements come before the coordinator's assignment: {turn}"
        );
        assert!(
            turn.contains(AUTHORITATIVE_RULE),
            "the child request states the operator request is authoritative over the \
             assignment: {turn}"
        );
    }
}

#[tokio::test]
async fn read_only_prompt_states_the_read_only_posture_and_no_mandates() {
    let provider = Provider::await_empty();
    let fixture = Fixture::with_endpoint("assignment-prompt", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("assignment-prompt").unwrap(),
        "Report an empty native child set".into(),
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();

    let bodies = provider.bodies.lock().unwrap();
    let first: Value = serde_json::from_str(bodies.first().expect("the root requested")).unwrap();
    let system = first["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "system")
        .and_then(|message| message["content"].as_str())
        .expect("the root request carries its system prompt");
    // A job with no approved write path is read-only: the prompt says so and
    // claims no checkout, edit, shell or owner candidate.
    assert!(
        system.contains(
            "This job is read-only: you can read and search the workspace, but you cannot write, \
             edit or delete files and you have no shell."
        ),
        "{system}"
    );
    assert!(
        system.contains("Do the work directly from what you can read. "),
        "{system}"
    );
    assert!(
        system.contains("Use `read`, `grep` and `glob` for files."),
        "{system}"
    );
    for gone in [
        "You have workspace tools over your own working checkout.",
        "The owner prepares the candidate and its readiness",
        "`edit` to change them",
        "Use `checklist` only when tracking several steps helps.",
        "checks",
        "read, list, glob, grep, edit, write, delete, and shell",
    ] {
        assert!(
            !system.contains(gone),
            "the read-only prompt must not claim this: {gone}"
        );
    }
    for mandate in [
        "at most 2048 bytes",
        "register the complete intended child assignment set",
        "Follow the next_action",
    ] {
        assert!(
            !system.contains(mandate),
            "the prompt must not carry the delegate-only mandate: {mandate}"
        );
    }
}
