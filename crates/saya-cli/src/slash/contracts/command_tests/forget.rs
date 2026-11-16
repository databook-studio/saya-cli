use super::*;

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
