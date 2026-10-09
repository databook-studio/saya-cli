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
