use super::*;

/// the trap that was removed, and the regression guard against its
/// return. Before the merge, `/contracts` and `/contract` were two *separate
/// operations* distinguished only by a trailing `s`: `/contracts` was `List`
/// and rejected any argument, `/contract` was `Show` and rejected none. A
/// user who mistyped the one letter they would not notice got a usage error
/// from the command they did not mean, and `closest_command` resolved the
/// near-miss to the *other* command — so it could not help. The before state
/// was captured by an earlier form of this test that asserted the asymmetry;
/// it passed against the unmerged code and failed once the merge landed,
/// proving the behaviour changed exactly as intended. This is the forward
/// guard: the one-letter difference must no longer select a different
/// operation or turn a valid argument into a usage error.
#[test]
fn s13_the_one_letter_no_longer_selects_a_different_operation() {
    // `/contracts <table>` is now Show, not the "takes no argument" error the
    // old List-only spelling raised — so a user who meant Show no longer gets a
    // usage error for supplying the very argument Show needs.
    assert_eq!(
        parse_contract_command("contracts", "analytics.public.orders")
            .unwrap()
            .unwrap(),
        ContractsCommand::Show {
            table: "analytics.public.orders".into(),
            profile: None,
        }
    );
    // `/contract` with no argument is now List, not the "missing argument"
    // error the old Show-only spelling raised — so a user who meant List no
    // longer gets a usage error for omitting the argument List takes none of.
    assert_eq!(
        parse_contract_command("contract", "").unwrap().unwrap(),
        ContractsCommand::List { profile: None }
    );
    // The two names no longer map to two different operations: for every
    // argument shape they produce the *same* command, so mistyping the one
    // letter cannot land you in a different operation.
    assert_eq!(
        parse_contract_command("contracts", "").unwrap().unwrap(),
        parse_contract_command("contract", "").unwrap().unwrap()
    );
    assert_eq!(
        parse_contract_command("contracts", "analytics.public.orders")
            .unwrap()
            .unwrap(),
        parse_contract_command("contract", "analytics.public.orders")
            .unwrap()
            .unwrap()
    );
}

#[test]
fn parse_contract_contracts_no_arg() {
    let cmd = parse_contract_command("contracts", "").unwrap().unwrap();
    assert_eq!(cmd, ContractsCommand::List { profile: None });
    // After the merge, `/contracts <table>` is Show, not an error.
    assert_eq!(
        parse_contract_command("contracts", "analytics.public.orders")
            .unwrap()
            .unwrap(),
        ContractsCommand::Show {
            table: "analytics.public.orders".into(),
            profile: None,
        }
    );
    // Two tokens is still a usage error: a qualified name is one token.
    assert!(parse_contract_command("contracts", "a b").is_err());
}

/// one name, the optional argument selects the operation —
/// what the CLI already does, and the same shape as `/queue [limit]`. No
/// argument is List; one token is Show. The trap is gone because there is
/// no second name to mistype into a different operation.
#[test]
fn contracts_one_name_arg_selects_the_operation() {
    // No argument → list every contract for the active profile.
    assert_eq!(
        parse_contract_command("contracts", "").unwrap().unwrap(),
        ContractsCommand::List { profile: None }
    );
    // One token → show that one object's contract.
    assert_eq!(
        parse_contract_command("contracts", "analytics.public.orders")
            .unwrap()
            .unwrap(),
        ContractsCommand::Show {
            table: "analytics.public.orders".into(),
            profile: None,
        }
    );
    // Two tokens is a usage error — a qualified name is a single token, and a
    // Show never takes two. The error is payload-free (no echo of the input).
    let bad = parse_contract_command("contracts", "a b").unwrap_err();
    assert!(!bad.0.contains("a b"));
    assert!(!bad.0.is_empty());
    // Surrounding whitespace does not turn one name into two.
    assert_eq!(
        parse_contract_command("contracts", "  analytics.public.orders  ")
            .unwrap()
            .unwrap(),
        ContractsCommand::Show {
            table: "analytics.public.orders".into(),
            profile: None,
        }
    );
}

/// `/contract` is kept as a silent alias of the merged command, not a
/// second operation. Whatever the argument, it routes to the same command
/// `/contracts` produces — so the two names can no longer disagree. (Kept as
/// an alias rather than removed so the TUI completion registry — mirrored by
/// `complete.rs` — stays in lockstep; see
/// the SPEC REVIEW.)
#[test]
fn contract_is_a_silent_alias_of_the_merged_command() {
    // `/contract <table>` does what `/contracts <table>` does: Show.
    assert_eq!(
        parse_contract_command("contract", "analytics.public.orders")
            .unwrap()
            .unwrap(),
        parse_contract_command("contracts", "analytics.public.orders")
            .unwrap()
            .unwrap()
    );
    // `/contract` with no argument does what `/contracts` does: List. It is no
    // longer the "missing argument" error the old Show-only spelling raised.
    assert_eq!(
        parse_contract_command("contract", "").unwrap().unwrap(),
        parse_contract_command("contracts", "").unwrap().unwrap()
    );
    // Two tokens is a usage error on both spellings.
    assert!(parse_contract_command("contract", "a b").is_err());
    assert!(parse_contract_command("contracts", "a b").is_err());
}

#[test]
fn parse_forget_one_id() {
    let cmd = parse_contract_command("forget", "abc-123")
        .unwrap()
        .unwrap();
    assert_eq!(
        cmd,
        ContractsCommand::Forget {
            claim_id: "abc-123".into(),
            reason: ForgetReasonArg::UserRequest,
        }
    );
    assert!(parse_contract_command("forget", "").is_err());
    assert!(parse_contract_command("forget", "a b").is_err());
}

#[test]
fn parse_queue_no_arg_is_default_limit() {
    let cmd = parse_contract_command("queue", "").unwrap().unwrap();
    assert_eq!(
        cmd,
        ContractsCommand::Queue {
            profile: None,
            limit: None
        }
    );
}

#[test]
fn parse_queue_optional_numeric_limit() {
    let cmd = parse_contract_command("queue", "20").unwrap().unwrap();
    assert_eq!(
        cmd,
        ContractsCommand::Queue {
            profile: None,
            limit: Some(20),
        }
    );
    // A non-numeric limit is a usage error that does not echo the input.
    let bad = parse_contract_command("queue", "lots").unwrap_err();
    assert!(!bad.0.contains("lots"));
    // Two tokens is a usage error.
    assert!(parse_contract_command("queue", "1 2").is_err());
}

#[test]
fn parse_confirm_one_prefix() {
    let cmd = parse_contract_command("confirm", "c-a86a3f")
        .unwrap()
        .unwrap();
    assert_eq!(
        cmd,
        ContractsCommand::Decide {
            prefix: "c-a86a3f".into(),
            decision: ReviewDecisionArg::Confirm,
            profile: None,
        }
    );
    // No token is a usage error that does not echo anything.
    let bad = parse_contract_command("confirm", "").unwrap_err();
    assert!(!bad.0.contains("c-"));
    assert!(!bad.0.is_empty());
    // Two tokens is a usage error that does not echo the input.
    let bad = parse_contract_command("confirm", "c-abc c-def").unwrap_err();
    assert!(!bad.0.contains("c-abc"));
    assert!(!bad.0.contains("c-def"));
}

#[test]
fn parse_reject_translates_to_decide() {
    assert_eq!(
        parse_contract_command("reject", "c-1").unwrap().unwrap(),
        ContractsCommand::Decide {
            prefix: "c-1".into(),
            decision: ReviewDecisionArg::Reject,
            profile: None,
        }
    );
    // Refuses a missing prefix without panicking.
    assert!(parse_contract_command("reject", "").is_err());
    // Refuses two tokens.
    assert!(parse_contract_command("reject", "a b").is_err());
}

/// `/use` is deliberately absent. `use_candidate_once` validates a claim but
/// the interactive session does not yet thread the admission into the next
/// recall, so the command could not do what its name promises. Shipping it
/// would have told a user their candidate was admitted when nothing had
/// changed — the exact overstatement this feature exists to avoid. It
/// returns here once the admission is threaded.
#[test]
fn use_is_not_a_contract_command_until_the_admission_is_threaded() {
    assert!(
        parse_contract_command("use", "c-1").unwrap().is_none(),
        "/use must not parse while it cannot admit anything"
    );
}

#[test]
fn parse_unknown_name_returns_none() {
    assert!(parse_contract_command("sql", "select 1").unwrap().is_none());
}

/// `/approve-all` defaults to the deny-by-default preview (no `--yes`),
/// `--yes` flips it, an optional number overrides the limit, and anything
/// else is a payload-free usage error.
#[test]
fn parse_approve_all_translates_to_the_batch_command() {
    assert_eq!(
        parse_contract_command("approve-all", "").unwrap().unwrap(),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: None,
            yes: false,
        }
    );
    assert_eq!(
        parse_contract_command("approve-all", "--yes")
            .unwrap()
            .unwrap(),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: None,
            yes: true,
        }
    );
    assert_eq!(
        parse_contract_command("approve-all", "--yes 10")
            .unwrap()
            .unwrap(),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: Some(10),
            yes: true,
        }
    );
    // A token that is neither `--yes` nor a number is a usage error that
    // does not echo it.
    let bad = parse_contract_command("approve-all", "kaboom").unwrap_err();
    assert!(!bad.0.contains("kaboom"));
    assert!(!bad.0.is_empty());
    // Two limits or two --yes flags are usage errors, not silent overrides.
    assert!(parse_contract_command("approve-all", "10 20").is_err());
    assert!(parse_contract_command("approve-all", "--yes --yes").is_err());
}
