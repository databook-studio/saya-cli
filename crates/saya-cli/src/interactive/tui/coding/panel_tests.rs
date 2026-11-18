use crate::cli::CodingCommand;
use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::transcript::BlockKind;

fn start() -> CodingCommand {
    CodingCommand::Start {
        id: "job-panel".into(),
        task: "edit the folder".into(),
        write_paths: vec![".".into()],
        database_profiles: Vec::new(),
        tools: Vec::new(),
        plan: false,
        approve: true,
        in_place: true,
        managed: false,
    }
}

/// The same start with its scope left empty, the shape the slash parser
/// produces when no `--write` is stated.
fn start_without_write() -> CodingCommand {
    let CodingCommand::Start {
        id,
        task,
        database_profiles,
        tools,
        plan,
        approve,
        in_place,
        managed,
        ..
    } = start()
    else {
        unreachable!("the fixture is a start");
    };
    CodingCommand::Start {
        id,
        task,
        write_paths: Vec::new(),
        database_profiles,
        tools,
        plan,
        approve,
        in_place,
        managed,
    }
}

/// A session start with no `--write` may edit the whole workspace, exactly
/// like `saya -p`; an explicit `--write` narrows it and is left alone.
#[test]
fn a_session_start_defaults_to_the_workspace() {
    let CodingCommand::Start { write_paths, .. } = start_without_write().with_session_write_scope()
    else {
        unreachable!("a start stays a start");
    };
    assert_eq!(write_paths, ["."], "no --write edits the whole workspace");

    let CodingCommand::Start { write_paths, .. } = start().with_session_write_scope() else {
        unreachable!("a start stays a start");
    };
    assert_eq!(write_paths, ["."], "the folder fixture keeps its scope");

    let explicit = {
        let CodingCommand::Start {
            id,
            task,
            database_profiles,
            tools,
            plan,
            approve,
            in_place,
            managed,
            ..
        } = start()
        else {
            unreachable!("the fixture is a start");
        };
        CodingCommand::Start {
            id,
            task,
            write_paths: vec!["src".to_owned()],
            database_profiles,
            tools,
            plan,
            approve,
            in_place,
            managed,
        }
    };
    let CodingCommand::Start { write_paths, .. } = explicit.with_session_write_scope() else {
        unreachable!("a start stays a start");
    };
    assert_eq!(
        write_paths,
        ["src"],
        "an explicit --write narrows the scope"
    );
}

/// A start shows the id it chose, so the user can name the job later; the
/// controls on an existing job name no new one.
#[test]
fn a_start_names_the_job_it_chose() {
    assert_eq!(
        super::started_line(&start()),
        Some("Started job job-panel".to_owned())
    );
    assert_eq!(
        super::started_line(&CodingCommand::Cancel {
            id: "job-panel".into()
        }),
        None
    );
}

/// Under an edit-capable mode the panel's own consent answers the approve
/// question and an empty `--write` scope means the folder; `ask` still asks.
#[test]
fn accept_edits_implies_the_panel_approval_and_the_folder_scope() {
    use saya_agent::ApprovalPolicy;
    assert!(super::worker::panel_consent(
        false,
        ApprovalPolicy::AcceptEdits
    ));
    assert!(super::worker::panel_consent(false, ApprovalPolicy::Bypass));
    assert!(!super::worker::panel_consent(false, ApprovalPolicy::Ask));
    assert!(super::worker::panel_consent(true, ApprovalPolicy::Ask));
    assert_eq!(
        crate::commands::coding::launch_write_scope(ApprovalPolicy::AcceptEdits, &[]),
        vec![".".to_owned()]
    );
}

/// A start with no bound workspace refuses before spawning anything: the
/// panel says why, and no worker runs.
#[test]
fn no_root_still_refuses() {
    let mut app = idle_app();
    app.coding_command(start(), None, saya_agent::ApprovalPolicy::Ask);
    assert!(app.coding_panel.is_none(), "no root, no panel");
    let block = app.transcript.blocks.last().expect("the refusal is shown");
    assert_eq!(block.kind, BlockKind::Error);
    assert!(
        block
            .text
            .contains("Bind a repository workspace before starting a job."),
        "the refusal names the missing binding: {}",
        block.text
    );
}
