//! The shell view parser and recorder: exactly one single-file view records a
//! witness, and every compound, redirected, option-shaped or out-of-tree
//! command records nothing.

use std::fs;
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolExecutor};
use saya_harness::runner::HostPolicy;
use saya_harness::workspace::Workspace;
use serde_json::json;

use super::super::{Background, ShellTool, compose_host_only};
use super::{ShellReadWitness, record, view_path};
use crate::agent::tools::DatabaseTools;
use crate::agent::tools::workspace::ReadWitnesses;

fn sandbox(label: &str) -> (std::path::PathBuf, Arc<Workspace>) {
    let root = std::env::temp_dir().join(format!("saya-view-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("sandbox root must create");
    let workspace = Arc::new(Workspace::open(&root).expect("workspace must open"));
    (root, workspace)
}

#[test]
fn the_five_single_file_views_are_recognised() {
    for command in [
        "cat notes.md",
        "nl notes.md",
        "head notes.md",
        "head -n 5 notes.md",
        "tail notes.md",
        "tail -n 5 notes.md",
        "sed -n '1,2p' notes.md",
    ] {
        assert_eq!(
            view_path(command).as_deref(),
            Some("notes.md"),
            "{command} is a single-file view"
        );
    }
}

#[test]
fn compound_redirected_and_option_shaped_commands_are_not_views() {
    for command in [
        "cat notes.md | head",
        "cat notes.md > copy.md",
        "cat notes.md >> copy.md",
        "cat notes.md; rm other.md",
        "tail -f notes.md",
        "tail -5 notes.md",
        "head -c 10 notes.md",
        "cat -n notes.md",
        "cat one.md two.md",
        "grep x notes.md",
        "sed 's/a/b/' notes.md",
        "sed -n '1p' notes.md",
        "cat $(pwd)/notes.md",
        "",
    ] {
        assert_eq!(view_path(command), None, "{command} must record nothing");
    }
}

#[test]
fn a_view_inside_the_workspace_records_the_current_digest() {
    let (root, workspace) = sandbox("inside");
    fs::write(root.join("notes.md"), b"hello anchor world\n").expect("fixture must write");
    let target = ShellReadWitness {
        witnesses: Arc::new(ReadWitnesses::default()),
        workspace: Arc::clone(&workspace),
    };

    record(&target, "cat notes.md");

    let (digest, size) = target
        .witnesses
        .snapshot("notes.md", &workspace)
        .expect("an in-tree view must record a witness");
    let expected = workspace.read("notes.md", 0).expect("fixture must read");
    assert_eq!(digest, expected.digest);
    assert_eq!(size, expected.size);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_view_outside_the_workspace_records_nothing() {
    let (root, workspace) = sandbox("outside");
    let outside = std::env::temp_dir().join(format!("saya-view-outside-{}", std::process::id()));
    fs::create_dir_all(&outside).expect("outside directory must create");
    fs::write(outside.join("secret.txt"), b"not yours\n").expect("fixture must write");
    let target = ShellReadWitness {
        witnesses: Arc::new(ReadWitnesses::default()),
        workspace: Arc::clone(&workspace),
    };

    record(&target, "cat ../secret.txt");
    record(&target, "cat /etc/passwd");

    assert!(
        target
            .witnesses
            .snapshot("../secret.txt", &workspace)
            .is_none(),
        "a path outside the workspace must not record"
    );
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn a_non_view_records_nothing_even_when_it_names_a_file() {
    let (root, workspace) = sandbox("non-view");
    fs::write(root.join("notes.md"), b"hello\n").expect("fixture must write");
    let target = ShellReadWitness {
        witnesses: Arc::new(ReadWitnesses::default()),
        workspace: Arc::clone(&workspace),
    };

    record(&target, "cat notes.md | head");
    record(&target, "cat notes.md > copy.md");
    record(&target, "tail -f notes.md");

    assert!(target.witnesses.snapshot("notes.md", &workspace).is_none());
    let _ = fs::remove_dir_all(root);
}

fn shell_over(root: &std::path::Path) -> ShellTool {
    compose_host_only(
        root,
        ApprovalPolicy::Bypass,
        None,
        HostPolicy::default(),
        CancellationToken::new(),
        Background::Off,
        None,
        saya_config::CommandRules::default(),
    )
    .expect("a host shell composes")
}

/// A successful `cat` in this run guards the later edit exactly as a `read`
/// does: no digest is copied through the model payload.
#[tokio::test]
async fn a_shell_view_guards_a_later_edit_without_a_digest() {
    let (root, workspace) = sandbox("shell-guards-edit");
    fs::write(root.join("notes.md"), b"hello anchor world\n").expect("fixture must write");
    let witnesses = Arc::new(ReadWitnesses::default());
    let shell =
        shell_over(&root).with_read_witnesses(Arc::clone(&witnesses), Arc::clone(&workspace));
    let viewed = shell
        .execute(
            "shell",
            json!({"command": "cat notes.md", "sandbox": false}),
        )
        .await
        .expect("the shell call runs");
    assert_eq!(viewed["exit_code"], 0, "{viewed}");

    let tools =
        DatabaseTools::for_child_checkout(Arc::clone(&workspace)).with_read_witnesses(witnesses);
    let edited = tools
        .execute(
            "edit",
            json!({"path": "notes.md", "old_text": "anchor", "new_text": "ANCHOR"}),
        )
        .await
        .expect("the shell view should guard this edit");
    assert!(
        edited.get("note").is_none(),
        "no re-anchor note is needed: {edited}"
    );
    assert_eq!(
        fs::read(root.join("notes.md")).expect("edited file must exist"),
        b"hello ANCHOR world\n"
    );
    let _ = fs::remove_dir_all(root);
}

/// A piped command is not a single-file view: it records nothing, so an edit
/// with no digest keeps the read-first refusal.
#[tokio::test]
async fn a_piped_shell_view_does_not_guard_a_later_edit() {
    let (root, workspace) = sandbox("shell-piped");
    fs::write(root.join("notes.md"), b"hello anchor world\n").expect("fixture must write");
    let witnesses = Arc::new(ReadWitnesses::default());
    let shell =
        shell_over(&root).with_read_witnesses(Arc::clone(&witnesses), Arc::clone(&workspace));
    shell
        .execute(
            "shell",
            json!({"command": "cat notes.md | head", "sandbox": false}),
        )
        .await
        .expect("the piped shell call runs");

    let tools =
        DatabaseTools::for_child_checkout(Arc::clone(&workspace)).with_read_witnesses(witnesses);
    let error = tools
        .execute(
            "edit",
            json!({"path": "notes.md", "old_text": "anchor", "new_text": "ANCHOR"}),
        )
        .await
        .expect_err("a piped view is not a read witness");
    assert!(
        matches!(error, saya_agent::ToolError::WorkspaceDigestRequired { .. }),
        "the read-first refusal is unchanged: {error}"
    );
    assert_eq!(
        fs::read(root.join("notes.md")).expect("file must survive"),
        b"hello anchor world\n"
    );
    let _ = fs::remove_dir_all(root);
}
