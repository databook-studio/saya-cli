use saya_agent::ApprovalPolicy;

#[test]
fn approval_policies_parse_from_cli_values() {
    assert_eq!(
        "ask".parse::<ApprovalPolicy>().unwrap(),
        ApprovalPolicy::Ask
    );
    assert_eq!(
        "read-only".parse::<ApprovalPolicy>().unwrap(),
        ApprovalPolicy::ReadOnly
    );
    assert_eq!(
        "never".parse::<ApprovalPolicy>().unwrap(),
        ApprovalPolicy::Never
    );
    assert!("writes".parse::<ApprovalPolicy>().is_err());
}
