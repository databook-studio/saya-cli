//! The over-bound shell ask's saved command: the modal names a private file
//! holding the exact bytes, and the file is gone once the answer lands.

use super::super::{ChannelApproval, StreamMsg};
use super::support::composed_facts;
use crate::agent::tools::shell_policy::definition;
use crate::grant_token::TurnPrimary;
use saya_agent::{
    ApprovalChoice, ApprovalDecider, ApprovalPolicy, CancellationToken, SessionPolicy,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

fn temp_state_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-approval-binding-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("the temp state dir is created");
    dir
}

fn saved_path(detail: &str) -> PathBuf {
    let after = detail
        .split("full command saved to ")
        .nth(1)
        .unwrap_or_else(|| panic!("the modal names the saved file: {detail}"));
    PathBuf::from(
        after
            .split(']')
            .next()
            .expect("the marker closes after the path"),
    )
}

fn assert_no_saved_command(dir: &Path) {
    assert!(
        crate::approval_facts::saved_commands(dir).is_empty(),
        "the answer removes the saved command"
    );
}

#[tokio::test]
async fn an_over_bound_shell_ask_removes_the_saved_command_after_the_answer() {
    let dir = temp_state_dir("answered");
    let journal = Arc::new(saya_store::SessionJournal::open(&dir));
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::with_cancellation(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        composed_facts(),
        Some(journal),
        CancellationToken::new(),
    );
    let command = format!("HEAD{}TAIL", "x".repeat(4_000));
    let expected = command.clone();
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest {
            detail, respond, ..
        }) = rx.recv().await
        {
            let detail = detail.expect("the shell ask carries facts");
            assert!(detail.contains("HEAD"), "the head is shown: {detail}");
            assert!(detail.contains("TAIL"), "the tail is shown: {detail}");
            assert!(
                detail.contains("bytes not shown"),
                "the withheld bytes are stated: {detail}"
            );
            let path = saved_path(&detail);
            assert_eq!(
                std::fs::read(&path).expect("the saved command exists"),
                expected.as_bytes(),
                "the saved file is the exact command"
            );
            let _ = respond.send(ApprovalChoice::AllowOnce);
        }
    });
    let arguments = serde_json::json!({"command": command});
    assert!(
        decider.approve(&definition(), &arguments).await,
        "the answer allows the command"
    );
    answerer.await.expect("the answerer completes");
    assert_no_saved_command(&dir);
    let _ = std::fs::remove_dir_all(&dir);
}
