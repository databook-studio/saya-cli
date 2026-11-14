//! The panel's shell approval: the operator is asked only for a host run — a
//! sandboxed call never reaches it — and the prompt opens with the subject line
//! the caller gave: the root's names its job, a helper's names the helper. The
//! prompt carries the command and the working directory
//! with no control or invisible bytes, a call too long to show in full is
//! refused unasked, and no answer is remembered between calls.

use super::{CodingMessage, egress_approval::PanelEgressApproval};
use crate::agent::tools::coding::plan_gate;
use crate::agent::tools::consent::{Caller, Operator};
use saya_agent::CancellationToken;
use saya_types::JobId;
use serde_json::{Value, json};
use std::sync::{Arc, mpsc};
use tokio::sync::oneshot;

/// The prompt line the command is shown on.
const COMMAND_LINE: &str = "$ ";
/// The subject line the root's shell asks under.
const HOST_LINE: &str = "Run a shell command for job job-a";
/// The sanitiser's per-field character budget, mirrored from the module.
const MAX_PROMPT_CHARS: usize = 2048;

fn panel(id: &str) -> (PanelEgressApproval, mpsc::Receiver<CodingMessage>) {
    let (sender, rx) = mpsc::sync_channel(4);
    (
        PanelEgressApproval {
            sender,
            token: CancellationToken::new(),
            id: id.into(),
        },
        rx,
    )
}

/// The next prompt the panel put to the operator, with its answer channel.
async fn prompt(rx: &mpsc::Receiver<CodingMessage>) -> (String, oneshot::Sender<bool>) {
    loop {
        match rx.try_recv() {
            Ok(CodingMessage::Approval { text, respond }) => return (text, respond),
            Ok(CodingMessage::Activity(_)) => continue,
            Err(_) => tokio::task::yield_now().await,
        }
    }
}

/// The command as the operator sees it: the `$ …` line, without its prefix.
fn shown_command(text: &str) -> &str {
    text.lines()
        .find_map(|line| line.strip_prefix(COMMAND_LINE))
        .expect("the prompt shows the command on its own line")
}

/// The subject is the caller's, so a helper's ask names the helper: the panel
/// shows whatever line it was handed, above the command.
#[tokio::test]
async fn the_prompt_opens_with_the_subject_the_caller_gave() {
    let (approval, rx) = panel("job-a");
    let subject = "Helper helper-2 wants to run a shell command for job job-a";
    let arguments = json!({"command":"cargo test"});
    let asked = tokio::spawn(async move { approval.ask(subject, &arguments).await });
    let (text, respond) = prompt(&rx).await;
    assert_eq!(
        text.lines().next(),
        Some(subject),
        "the prompt names whoever is asking: {text:?}"
    );
    respond.send(true).unwrap();
    assert!(asked.await.unwrap());
}

/// A plan-first job's `submit_plan` reaches the same panel as the gate's own
/// line, with the plan itself as the target — sanitised like every other
/// model-authored target — so the operator answers for the text it reads.
#[tokio::test]
async fn a_submitted_plan_is_shown_as_the_plan() {
    let (approval, rx) = panel("job-a");
    let subject = plan_gate::line(&Caller::Root {
        job_id: JobId::parse("job-a").unwrap(),
    });
    let arguments = json!({"plan": "1. change \u{202E}src/lib.rs\u{2066}\n2. cargo test"});
    let asked = {
        let subject = subject.clone();
        tokio::spawn(async move { approval.ask(&subject, &arguments).await })
    };
    let (text, respond) = prompt(&rx).await;
    assert_eq!(
        text.lines().next(),
        Some(subject.as_str()),
        "the prompt opens with the gate's own line: {text:?}"
    );
    assert!(text.contains("1. change \u{FFFD}src/lib.rs\u{FFFD}"));
    assert!(text.contains("2. cargo test"));
    assert!(text.contains("/approve job-a"));
    for raw in INVISIBLE {
        assert!(
            !text.contains(raw),
            "the raw {raw:?} must not reach the operator's screen: {text:?}"
        );
    }
    respond.send(true).unwrap();
    assert!(asked.await.unwrap());
}

/// A plan the prompt cannot paint in full is refused unasked, exactly as an
/// over-long command is: the operator never approves text it cannot read. The
/// gate caps a plan at the same number of characters, so this guard is the
/// panel's own.
#[tokio::test]
async fn a_plan_longer_than_the_prompt_shows_is_denied_without_asking() {
    let (approval, rx) = panel("job-a");
    let subject = plan_gate::line(&Caller::Root {
        job_id: JobId::parse("job-a").unwrap(),
    });
    assert!(
        !approval
            .ask(&subject, &json!({"plan": "x".repeat(MAX_PROMPT_CHARS + 1)}))
            .await
    );
    assert!(
        rx.try_recv().is_err(),
        "the operator is never asked about a plan it cannot fully see"
    );
}

#[tokio::test]
async fn a_call_without_a_command_is_refused_without_asking() {
    let (approval, rx) = panel("job-a");
    assert!(!approval.ask(HOST_LINE, &json!({})).await);
    assert!(!approval.ask(HOST_LINE, &json!({"command": 7})).await);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn every_call_is_asked_again_and_follows_the_answer() {
    let (approval, rx) = panel("job-a");
    let approval = Arc::new(approval);
    let arguments = json!({"command":"cargo test","workdir":"crates"});
    let first = {
        let approval = Arc::clone(&approval);
        let arguments = arguments.clone();
        tokio::spawn(async move { approval.ask(HOST_LINE, &arguments).await })
    };
    let (text, respond) = prompt(&rx).await;
    assert_eq!(
        text.lines().next(),
        Some(HOST_LINE),
        "the prompt names the job it is asking about: {text:?}"
    );
    assert!(text.contains("cargo test"));
    assert!(text.contains("in: crates"));
    assert!(text.contains("/approve job-a"));
    respond.send(true).unwrap();
    assert!(first.await.unwrap());
    assert!(rx.try_recv().is_err(), "one prompt per call, no more");

    // A second identical call asks again: the panel remembers nothing.
    let second = {
        let approval = Arc::clone(&approval);
        let arguments = arguments.clone();
        tokio::spawn(async move { approval.ask(HOST_LINE, &arguments).await })
    };
    let (_text, respond) = prompt(&rx).await;
    respond.send(false).unwrap();
    assert!(!second.await.unwrap(), "the first answer is never reused");
}

#[tokio::test]
async fn a_denial_a_dropped_answer_and_a_cancellation_all_refuse() {
    let (approval, rx) = panel("job-a");
    let approval = Arc::new(approval);
    let arguments = json!({"command":"cargo test"});

    let asked = {
        let approval = Arc::clone(&approval);
        let arguments = arguments.clone();
        tokio::spawn(async move { approval.ask(HOST_LINE, &arguments).await })
    };
    let (_text, respond) = prompt(&rx).await;
    respond.send(false).unwrap();
    assert!(!asked.await.unwrap());

    let asked = {
        let approval = Arc::clone(&approval);
        let arguments = arguments.clone();
        tokio::spawn(async move { approval.ask(HOST_LINE, &arguments).await })
    };
    let (_text, respond) = prompt(&rx).await;
    drop(respond);
    assert!(!asked.await.unwrap(), "a dropped responder is a refusal");

    approval.token.cancel();
    assert!(!approval.ask(HOST_LINE, &arguments).await);
}

#[tokio::test]
async fn control_bytes_never_reach_the_operator_screen() {
    let (approval, rx) = panel("job-a");
    let command = "printf 'a\u{1b}[2Jb\rc\u{0}d'";
    let arguments = json!({"command": command});
    let asked = tokio::spawn(async move { approval.ask(HOST_LINE, &arguments).await });
    let (text, respond) = prompt(&rx).await;
    for (name, raw) in [("ESC", '\u{1b}'), ("CR", '\r'), ("NUL", '\u{0}')] {
        assert!(
            !text.contains(raw),
            "the raw {name} byte must not reach the operator's screen"
        );
    }
    assert!(
        shown_command(&text).contains("a\u{FFFD}[2Jb"),
        "the control byte is shown, replaced: {}",
        shown_command(&text)
    );
    respond.send(false).unwrap();
    assert!(!asked.await.unwrap());
}

#[tokio::test]
async fn a_command_longer_than_the_prompt_shows_is_denied_without_asking() {
    let (approval, rx) = panel("job-a");
    let arguments = json!({"command": "x".repeat(MAX_PROMPT_CHARS + 1)});
    assert!(!approval.ask(HOST_LINE, &arguments).await);
    assert!(
        rx.try_recv().is_err(),
        "the operator is never asked about a command it cannot fully see"
    );

    // Exactly the budget is still asked: the operator sees every character.
    let arguments = json!({"command": "x".repeat(MAX_PROMPT_CHARS)});
    let asked = tokio::spawn(async move { approval.ask(HOST_LINE, &arguments).await });
    let (text, respond) = prompt(&rx).await;
    assert_eq!(shown_command(&text).chars().count(), MAX_PROMPT_CHARS);
    respond.send(true).unwrap();
    assert!(asked.await.unwrap());
}

/// The invisible characters a terminal renders as nothing while they steer
/// what the operator reads: bidi overrides and isolates, zero-width joiners
/// and marks, and the byte-order mark.
const INVISIBLE: [char; 7] = [
    '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{200B}', '\u{FEFF}',
];

#[tokio::test]
async fn invisible_format_characters_never_reach_the_operator_screen() {
    let (approval, rx) = panel("job-a");
    let command = "echo \u{202E}rm\u{2066} -rf\u{2067} /\u{2068}now\u{2069}\u{200B}\u{FEFF}";
    let arguments = json!({"command": command});
    let asked = tokio::spawn(async move { approval.ask(HOST_LINE, &arguments).await });
    let (text, respond) = prompt(&rx).await;
    for raw in INVISIBLE {
        assert!(
            !text.contains(raw),
            "the raw {raw:?} must not reach the operator's screen: {text:?}"
        );
    }
    assert!(shown_command(&text).contains("echo \u{FFFD}rm"));
    respond.send(false).unwrap();
    assert!(!asked.await.unwrap());
}

#[tokio::test]
async fn an_invisible_workdir_is_shown_without_its_format_characters() {
    let (approval, rx) = panel("job-a");
    let arguments = json!({"command":"cargo test","workdir": "\u{202E}crates\u{2066}/\u{200B}"});
    let asked = tokio::spawn(async move { approval.ask(HOST_LINE, &arguments).await });
    let (text, respond) = prompt(&rx).await;
    for raw in INVISIBLE {
        assert!(
            !text.contains(raw),
            "the raw {raw:?} must not reach the operator's screen: {text:?}"
        );
    }
    assert!(text.contains("in: \u{FFFD}crates\u{FFFD}/\u{FFFD}"));
    respond.send(false).unwrap();
    assert!(!asked.await.unwrap());
}

#[tokio::test]
async fn a_full_queue_refuses_without_waiting_for_the_operator() {
    let (sender, rx) = mpsc::sync_channel(1);
    sender
        .try_send(CodingMessage::Activity("queued".into()))
        .unwrap();
    let approval = PanelEgressApproval {
        sender,
        token: CancellationToken::new(),
        id: "job-a".into(),
    };
    let arguments: Value = json!({"command":"cargo test"});
    assert!(!approval.ask(HOST_LINE, &arguments).await);
    assert!(matches!(rx.try_recv(), Ok(CodingMessage::Activity(text)) if text == "queued"));
}
