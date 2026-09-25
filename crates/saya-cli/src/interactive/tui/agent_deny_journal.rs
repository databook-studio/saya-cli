/// Deny preemption plus prompt journaling: preempted grants, the stdin gate, journal-once and journal-failure.
/// Moved byte-identical from the hub; no snapshots involved.
use super::super::{ChannelApproval, StreamMsg};
use super::support::{composed_facts, side_effecting_tool};
use crate::approval_facts::ApprovalFacts;
use crate::grant_token::TurnPrimary;
use crate::prompt_approval::TerminalApproval;
use saya_agent::{
    ApprovalChoice, ApprovalDecider, ApprovalDecision, ApprovalPolicy, SessionPolicy,
};
use tokio::sync::mpsc::unbounded_channel;

/// A denied name preempts a held grant through the policy engine: the grant
/// resolves `Allow`, but the decider still refuses without asking — in every
/// mode, bypass included. The ask never renders (the channel is closed, so
/// an ask would deny), and the refusal the decider words for the loop is the
/// deny bytes, not the engine's generic denial.
#[tokio::test]
async fn a_denied_name_preempts_a_held_grant_without_asking() {
    use crate::interactive::session_definitions;
    for mode in [ApprovalPolicy::Ask, ApprovalPolicy::Bypass] {
        let tool = session_definitions::run_command();
        let arguments = serde_json::json!({"program": "curl"});
        let policy = SessionPolicy::new(mode);
        policy.grants().grant("command:curl");
        let mut facts = composed_facts();
        facts.host = Some(crate::approval_facts::HostFacts::for_tests());
        facts.denied_programs = vec!["curl".to_owned()];
        assert_eq!(
            policy.resolve(&tool.effect, Some("command:curl")),
            ApprovalDecision::Allow,
            "the held grant resolves Allow under {mode:?}: deny preempts after the engine"
        );
        // The terminal decider refuses without prompting.
        let terminal = TerminalApproval::from_session(
            policy.clone(),
            mode == ApprovalPolicy::Ask,
            TurnPrimary::default(),
            facts.clone(),
            None,
        );
        assert!(
            !terminal.approve(&tool, &arguments).await,
            "a denied name refuses under {mode:?} even with the grant held"
        );
        assert!(
            terminal
                .refusal_detail(&tool, &arguments)
                .is_some_and(|detail| detail.contains("deny list")
                    && detail.contains("allowed programs may still invoke it")),
            "the terminal decider words the denial with the typed refusal under {mode:?}"
        );
        // The TUI decider refuses the same way: a closed channel means an
        // ask would deny, and the preemption answers before any ask renders.
        let (tx, rx) = unbounded_channel();
        drop(rx);
        let channel = ChannelApproval::new(
            tx,
            policy.clone(),
            TurnPrimary::default(),
            facts.clone(),
            None,
        );
        assert!(
            !channel.approve(&tool, &arguments).await,
            "the TUI refuses a denied name under {mode:?} even with the grant held"
        );
        assert!(
            channel
                .refusal_detail(&tool, &arguments)
                .is_some_and(|detail| detail.contains("deny list")
                    && detail.contains("allowed programs may still invoke it")),
            "the TUI decider words the denial with the typed refusal under {mode:?}"
        );
    }
}

/// The stdin gate the split must never move: `TerminalApproval` with
/// `can_prompt = false` denies an ungranted ask without reading stdin — the
/// same answer as before, on the same flag. The TUI's modal path answers
/// `true` for the approval-surface flag, which this decider never reads.
/// Pinned here because widening the gate to the approval flag would let a
/// headless surface read stdin it must not touch.
#[tokio::test]
async fn the_stdin_gate_still_denies_without_reading_stdin() {
    let tool = side_effecting_tool();
    let arguments = serde_json::json!({});
    // `can_prompt = false` is the TUI's stdin answer too: deny, and deny
    // without touching stdin (this test would block on a read if it did).
    let no_stdin = TerminalApproval::new(
        ApprovalPolicy::Ask,
        false,
        TurnPrimary::default(),
        ApprovalFacts::default(),
    );
    assert!(
        !no_stdin.approve(&tool, &arguments).await,
        "an ungranted ask with no stdin surface denies"
    );
    // The mode denials never reach the gate: `never` and `read-only` deny
    // before any surface question arises.
    for mode in [ApprovalPolicy::Never, ApprovalPolicy::ReadOnly] {
        let decider = TerminalApproval::new(
            mode,
            false,
            TurnPrimary::default(),
            ApprovalFacts::default(),
        );
        assert!(
            !decider.approve(&tool, &arguments).await,
            "{mode:?} denies regardless of the stdin surface"
        );
    }
}

/// The session journal's `prompt` properties, driven through the one ask
/// surface a test can drive (the TUI's channel; the terminal decider shares
/// the operation, `record_prompt_answer`):
///
/// - a first `[s]` grant writes exactly one line — `source: "prompt"` — and
///   the line is already on disk when `approve` returns true, which is the
///   moment the call it allowed is allowed to run: the write precedes the
///   run gate opening, so the record is meaningful;
/// - a second grant of the same token writes none: the store answers
///   "already", and the journal-once hook is that answer.
#[tokio::test]
async fn a_prompted_grant_journals_once_before_the_call_it_allowed_runs() {
    let dir = std::env::temp_dir().join(format!("saya-tui-journal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    let journal = std::sync::Arc::new(saya_store::SessionJournal::open(&dir));
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        composed_facts(),
        Some(journal.clone()),
    );
    let tool = crate::interactive::session_definitions::workspace_write();
    let arguments = serde_json::json!({"path": "notes.md", "content": "hello"});
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(grant.as_deref(), Some("workspace-write"));
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
    });
    assert!(
        decider.approve(&tool, &arguments).await,
        "the user's session grant allows the call that asked"
    );
    answerer.await.expect("the answerer completes");
    // The line already exists at the moment the grant opened the run gate:
    // this is the ordering property, pinned at the only instant the test can
    // observe — the call it allowed is about to run.
    assert_eq!(
        journal.read().expect("the journal reads"),
        vec![saya_store::JournalEvent::Granted {
            token: "workspace-write".to_owned(),
            source: saya_store::GrantSource::Prompt,
        }],
        "one line, source prompt, written before the call it allowed runs"
    );
    // The second call of the same shape resolves Allow through the grant —
    // no ask, no new record, no line.
    assert!(
        decider.approve(&tool, &arguments).await,
        "the granted token pre-answers the next call of the same shape"
    );
    assert_eq!(
        journal.read().expect("the journal reads").len(),
        1,
        "a second grant of the same token writes none"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A journal write that fails must not take the session down — the user's
/// `[s]` stands and the call it allowed runs — and must not fail silently:
/// the decider says the missing audit line into the transcript.
#[tokio::test]
async fn a_failed_journal_write_says_so_and_does_not_take_the_call_down() {
    let dir = std::env::temp_dir().join(format!("saya-tui-journal-fail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    // The journal path is a directory, so every append fails.
    std::fs::create_dir_all(dir.join("journal.ndjson")).expect("the block is made");
    let journal = std::sync::Arc::new(saya_store::SessionJournal::open(&dir));
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        composed_facts(),
        Some(journal),
    );
    let tool = crate::interactive::session_definitions::workspace_write();
    let arguments = serde_json::json!({"path": "notes.md", "content": "hello"});
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(grant.as_deref(), Some("workspace-write"));
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
        // Then the warning the decider said arrives on the same channel.
        match rx.recv().await {
            Some(StreamMsg::Notice(warning)) => Some(warning),
            Some(_) => panic!("the decider said something other than the journal warning"),
            None => panic!("the decider said nothing"),
        }
    });
    assert!(
        decider.approve(&tool, &arguments).await,
        "the consent stands: a failed audit write does not revoke it"
    );
    let warning = answerer
        .await
        .expect("the answerer completes")
        .expect("the decider said the warning");
    assert!(
        warning.to_lowercase().contains("journal"),
        "the warning names the journal: {warning}"
    );
    // The grant is in force: the next call of the same shape resolves Allow
    // through it — the session carries on.
    assert!(
        decider.approve(&tool, &arguments).await,
        "the granted token pre-answers the next call: the session carries on"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
