use super::*;

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
