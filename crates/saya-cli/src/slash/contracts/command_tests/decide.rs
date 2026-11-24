use super::*;

#[test]
fn parse_confirm_one_prefix() {
    assert_eq!(
        command_of("/confirm c-a86a3f"),
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
        command_of("/reject c-1"),
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

/// The usage line names the spelling the user typed: a deprecated `/confirm`
/// says `/confirm <claim-id-prefix>`, the canonical form says `/contracts
/// confirm <claim-id-prefix>`. Neither echoes the prefix.
#[test]
fn the_usage_line_names_the_spelling_typed() {
    let alias = parse_contract_command("confirm", "").unwrap_err();
    assert_eq!(alias.0, "/confirm <claim-id-prefix>");
    let canonical = parse_contract_command("contracts", "confirm").unwrap_err();
    assert_eq!(canonical.0, "/contracts confirm <claim-id-prefix>");
}
