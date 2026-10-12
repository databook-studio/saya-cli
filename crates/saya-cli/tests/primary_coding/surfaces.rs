use super::support::Fixture;
use clap::Parser;
use saya_agent::CancellationToken;
use saya_cli::{
    Cli, CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, Command, SlashCommand,
    dispatch_coding, parse_slash_command,
};
use saya_types::JobId;
use std::time::Duration;

async fn create_job(fixture: &Fixture, job: &str) -> JobId {
    let request = CodingJobRequest::new(
        JobId::parse(job).unwrap(),
        "inspect only".into(),
        vec![],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let runtime = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    runtime.job_id().clone()
}

fn cli_parse<const N: usize>(words: [&str; N]) -> CodingCommand {
    let parsed = Cli::try_parse_from(["saya", "coding"].into_iter().chain(words)).unwrap();
    match parsed.command {
        Some(Command::Coding { command, .. }) => command,
        _ => panic!("coding command"),
    }
}

fn slash_parse(input: &str) -> CodingCommand {
    let parsed = parse_slash_command(input).unwrap().unwrap();
    match parsed {
        SlashCommand::Coding(command) => command,
        _ => panic!("coding command"),
    }
}

async fn list_job_ids(fixture: &Fixture) -> Vec<String> {
    let listed = dispatch_coding(&fixture.application, CodingCommand::List { after: None })
        .await
        .unwrap();
    listed["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|job| job["job_id"].as_str().unwrap().to_string())
        .collect()
}

fn names_approve(refusal: &CodingError) {
    assert!(
        refusal.to_string().contains("--approve"),
        "the refusal names the approving action: {refusal}"
    );
}

#[tokio::test]
async fn cancel_through_cli_and_slash_records_the_same_single_request() {
    let fixture = Fixture::new("surface-cancel").await;
    let job = create_job(&fixture, "job-cancel-a").await;
    let id = job.as_str();
    let cli = cli_parse(["cancel", id]);
    let slash = slash_parse(&format!("/coding cancel {id}"));
    assert_eq!(cli, slash);
    let first = dispatch_coding(&fixture.application, cli).await.unwrap();
    assert_eq!(first["job_id"], id);
    assert_eq!(first["cancel_requested"], true);
    assert_eq!(first["new_request"], true);
    let show = cli_parse(["show", id]);
    assert_eq!(show, slash_parse(&format!("/coding show {id}")));
    let before = dispatch_coding(&fixture.application, show).await.unwrap();
    let second = dispatch_coding(&fixture.application, slash).await.unwrap();
    assert_eq!(second["job_id"], id);
    assert_eq!(second["cancel_requested"], true);
    assert_eq!(second["new_request"], false);
    let after = dispatch_coding(
        &fixture.application,
        slash_parse(&format!("/coding show {id}")),
    )
    .await
    .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn a_denied_headless_start_returns_promptly_with_the_approval_refusal() {
    let fixture = Fixture::new("surface-start-denied").await;
    let denied = CodingCommand::Start {
        id: "job-denied-start".into(),
        task: "bounded task".into(),
        write_paths: vec![],
        programs: vec![],
        approve: false,
    };
    let denial = tokio::time::timeout(
        Duration::from_secs(10),
        dispatch_coding(&fixture.application, denied),
    )
    .await
    .expect("a denied start must complete promptly instead of hanging");
    let Err(refusal) = denial else {
        panic!("a denied start must be refused");
    };
    assert!(matches!(refusal, CodingError::ApprovalRequired));
    names_approve(&refusal);
    assert_eq!(list_job_ids(&fixture).await, Vec::<String>::new());
    let approved = CodingCommand::Start {
        id: "job-approved-start".into(),
        task: "bounded task".into(),
        write_paths: vec![],
        programs: vec![],
        approve: true,
    };
    // The approved start runs its job after creating it and may fail in this
    // fixture when no provider answers; creation itself is what the control pins.
    let _ = dispatch_coding(&fixture.application, approved).await;
    assert_eq!(
        list_job_ids(&fixture).await,
        vec!["job-approved-start".to_string()]
    );
}

#[tokio::test]
async fn resume_without_approval_is_refused_for_both_surfaces_and_changes_nothing() {
    let fixture = Fixture::new("surface-resume-denied").await;
    let job = create_job(&fixture, "job-resume-a").await;
    let id = job.as_str();
    let expected = CodingCommand::Resume {
        id: id.into(),
        approve: false,
    };
    let cli = cli_parse(["resume", id]);
    let slash = slash_parse(&format!("/coding resume {id}"));
    assert_eq!(cli, expected);
    assert_eq!(slash, expected);
    let show = cli_parse(["show", id]);
    let before = dispatch_coding(&fixture.application, show).await.unwrap();
    for command in [cli, slash] {
        let Err(refusal) = dispatch_coding(&fixture.application, command).await else {
            panic!("a resume without approval must be refused");
        };
        assert!(matches!(refusal, CodingError::ApprovalRequired));
        names_approve(&refusal);
    }
    let after = dispatch_coding(
        &fixture.application,
        slash_parse(&format!("/coding show {id}")),
    )
    .await
    .unwrap();
    assert_eq!(after, before);
}
