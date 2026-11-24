//! The write ask's mode arms, and the order it decides in: the capability
//! first — a call it refuses is refused in every mode, with nobody asked — and
//! then the job's mode for a write-shaped call it admitted.

use saya_agent::ApprovalPolicy;

use super::consent::Answer;
use super::write_ask_tests::{Capability, Recording, ask, call, definition};

/// `read-only` and `never` refuse a write, and nobody is asked.
#[tokio::test]
async fn read_only_and_never_refuse_without_a_question() {
    for mode in [ApprovalPolicy::ReadOnly, ApprovalPolicy::Never] {
        let operator = Recording::new(Answer::Once);
        let ask = ask(mode, operator.clone());

        assert!(
            !ask.approve(&Capability::new(true), &definition("delete"), &call())
                .await,
            "{mode:?} refuses a write"
        );
        assert_eq!(operator.calls(), 0, "{mode:?} asks nobody");
    }
}

/// `accept-edits` and `bypass` admit a write without asking.
#[tokio::test]
async fn accept_edits_and_bypass_admit_without_a_question() {
    for mode in [ApprovalPolicy::AcceptEdits, ApprovalPolicy::Bypass] {
        let operator = Recording::new(Answer::Deny);
        let ask = ask(mode, operator.clone());

        assert!(
            ask.approve(&Capability::new(true), &definition("write"), &call())
                .await,
            "{mode:?} admits"
        );
        assert_eq!(operator.calls(), 0, "{mode:?} asks nobody");
    }
}

/// The capability decides first: a call it refuses is refused in every mode,
/// and the operator is never asked about it.
#[tokio::test]
async fn a_call_the_capability_refuses_never_reaches_the_operator() {
    for mode in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
        ApprovalPolicy::Bypass,
    ] {
        let operator = Recording::new(Answer::Once);
        let ask = ask(mode, operator.clone());
        let capability = Capability::new(false);

        assert!(
            !ask.approve(&capability, &definition("write"), &call())
                .await,
            "{mode:?} keeps the capability's refusal"
        );
        assert_eq!(operator.calls(), 0, "{mode:?} asks nobody");
        assert_eq!(capability.seen(), 1, "the capability was consulted");
    }
}

/// A call that is not write-shaped keeps the capability's own answer and its
/// own ask, whatever the job's mode says: a read is not this question.
#[tokio::test]
async fn a_read_keeps_the_capabilitys_answer() {
    let operator = Recording::new(Answer::Deny);
    let ask = ask(ApprovalPolicy::Ask, operator.clone());

    assert!(
        ask.approve(&Capability::new(true), &definition("read"), &call())
            .await
    );
    assert_eq!(
        ask.would_ask(&Capability::new(true), &definition("read"), &call()),
        None,
        "a read carries no ask of this family"
    );
    assert_eq!(operator.calls(), 0);
}
