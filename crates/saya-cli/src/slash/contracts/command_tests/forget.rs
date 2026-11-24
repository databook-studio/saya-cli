use super::*;

#[test]
fn parse_forget_one_id() {
    assert_eq!(
        command_of("/forget abc-123"),
        ContractsCommand::Forget {
            claim_id: "abc-123".into(),
            reason: ForgetReasonArg::UserRequest,
        }
    );
    assert!(parse_contract_command("forget", "").is_err());
    assert!(parse_contract_command("forget", "a b").is_err());
}

/// `/remember` and `/forget` are the knowledge store's own short verbs, not
/// deprecated spellings: neither states a note.
#[test]
fn the_short_verbs_state_no_note() {
    let remember = parse_contract_command("remember", "analytics.public.orders alias orders")
        .unwrap()
        .unwrap();
    assert_eq!(remember.note, None, "/remember is not deprecated");
    let forget = parse_contract_command("forget", "abc-123")
        .unwrap()
        .unwrap();
    assert_eq!(forget.note, None, "/forget is not deprecated");
}
