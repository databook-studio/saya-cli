use super::*;

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
