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
