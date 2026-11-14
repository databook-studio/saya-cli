use super::*;

/// The one-letter trap, and the regression guard against its return.
/// `/contracts` and `/contract` are two spellings distinguished only by a
/// trailing `s`; a user who mistypes one letter must not land in a different
/// operation or turn a valid argument into a usage error. Both names map to
/// the same operation for every argument shape, so `closest_command`
/// resolving the near-miss to the other spelling cannot mislead.
#[test]
fn s13_the_one_letter_no_longer_selects_a_different_operation() {
    // `/contracts <table>` is Show, so a user who meant Show does not get a
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
    // `/contract` with no argument is List, so a user who meant List does not
    // get a usage error for omitting an argument List does not take.
    assert_eq!(
        parse_contract_command("contract", "").unwrap().unwrap(),
        ContractsCommand::List { profile: None }
    );
    // The two names do not map to two different operations: for every
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
    // `/contracts <table>` is Show, not an error.
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

/// With one name, the optional argument selects the operation — the same
/// shape as `/queue [limit]`. No argument is List; one token is Show. There
/// is no second name to mistype into a different operation.
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

/// `/contract` is a silent alias of the same command, not a second operation.
/// Whatever the argument, it routes to the same command
/// `/contracts` produces — so the two names cannot disagree. (Kept as
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
    // the "missing argument" error the Show-only spelling raised.
    assert_eq!(
        parse_contract_command("contract", "").unwrap().unwrap(),
        parse_contract_command("contracts", "").unwrap().unwrap()
    );
    // Two tokens is a usage error on both spellings.
    assert!(parse_contract_command("contract", "a b").is_err());
    assert!(parse_contract_command("contracts", "a b").is_err());
}
