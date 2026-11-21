use super::*;

/// The one-letter trap, and the regression guard against its return.
/// `/contracts` and `/contract` are two spellings distinguished only by a
/// trailing `s`; a user who mistypes one letter must not land in a different
/// operation or turn a valid argument into a usage error. `/contract` is now
/// the one-release alias, so the two names map to the same operation for every
/// argument shape — and only the alias states a note.
#[test]
fn s13_the_one_letter_no_longer_selects_a_different_operation() {
    // `/contracts <table>` is Show, so a user who meant Show does not get a
    // usage error for supplying the very argument Show needs.
    assert_eq!(
        command_of("/contracts analytics.public.orders"),
        ContractsCommand::Show {
            table: "analytics.public.orders".into(),
            profile: None,
        }
    );
    // `/contract` with no argument is List, so a user who meant List does not
    // get a usage error for omitting an argument List does not take.
    assert_eq!(
        command_of("/contract"),
        ContractsCommand::List { profile: None }
    );
    // The two names do not map to two different operations: for every argument
    // shape they produce the *same* command, so mistyping the one letter cannot
    // land you in a different operation.
    for arg in ["", "analytics.public.orders", "list", "queue 5"] {
        assert_eq!(
            command_of(&format!("/contracts {arg}")),
            command_of(&format!("/contract {arg}")),
            "the two spellings disagree on {arg:?}"
        );
    }
    // The alias is the one that says so.
    let alias = parse_contract_command("contract", "").unwrap().unwrap();
    assert!(alias.note.is_some(), "/contract must state its note");
    let canonical = parse_contract_command("contracts", "").unwrap().unwrap();
    assert_eq!(canonical.note, None, "/contracts is the canonical spelling");
}

/// With one name, the first token selects the operation: absent → list, a verb
/// word → that verb, a lone table → show. Two tokens that name no verb is still
/// a usage error, and it never echoes the tail.
#[test]
fn contracts_first_token_selects_the_operation() {
    assert_eq!(
        command_of("/contracts"),
        ContractsCommand::List { profile: None }
    );
    assert_eq!(
        command_of("/contracts analytics.public.orders"),
        ContractsCommand::Show {
            table: "analytics.public.orders".into(),
            profile: None,
        }
    );
    let bad = parse_contract_command("contracts", "a b").unwrap_err();
    assert!(!bad.0.contains("a b"));
    assert!(!bad.0.is_empty());
    // Surrounding whitespace does not turn one name into two.
    assert_eq!(
        command_of("/contracts  analytics.public.orders  "),
        ContractsCommand::Show {
            table: "analytics.public.orders".into(),
            profile: None,
        }
    );
}
