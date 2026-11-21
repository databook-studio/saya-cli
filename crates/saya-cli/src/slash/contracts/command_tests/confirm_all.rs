use super::*;

/// `/contracts confirm-all` is the batch approve: it previews the queue and
/// approves nothing without `--yes` (deny by default), `--yes` approves, an
/// optional number overrides the limit, and anything else is a payload-free
/// usage error — the grammar `/approve-all` has always had.
#[test]
fn confirm_all_previews_until_yes() {
    assert_eq!(
        command_of("/contracts confirm-all"),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: None,
            yes: false,
        }
    );
    assert_eq!(
        command_of("/contracts confirm-all --yes"),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: None,
            yes: true,
        }
    );
    assert_eq!(
        command_of("/contracts confirm-all 3"),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: Some(3),
            yes: false,
        }
    );
    let bad = parse_contract_command("contracts", "confirm-all kaboom").unwrap_err();
    assert!(!bad.0.contains("kaboom"));
    assert!(parse_contract_command("contracts", "confirm-all 1 2").is_err());
    assert!(parse_contract_command("contracts", "confirm-all --yes --yes").is_err());
}
