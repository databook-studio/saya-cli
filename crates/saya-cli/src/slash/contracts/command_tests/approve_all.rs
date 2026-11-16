use super::*;

/// `/approve-all` defaults to the deny-by-default preview (no `--yes`),
/// `--yes` flips it, an optional number overrides the limit, and anything
/// else is a payload-free usage error.
#[test]
fn parse_approve_all_translates_to_the_batch_command() {
    assert_eq!(
        parse_contract_command("approve-all", "").unwrap().unwrap(),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: None,
            yes: false,
        }
    );
    assert_eq!(
        parse_contract_command("approve-all", "--yes")
            .unwrap()
            .unwrap(),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: None,
            yes: true,
        }
    );
    assert_eq!(
        parse_contract_command("approve-all", "--yes 10")
            .unwrap()
            .unwrap(),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: Some(10),
            yes: true,
        }
    );
    // A token that is neither `--yes` nor a number is a usage error that
    // does not echo it.
    let bad = parse_contract_command("approve-all", "kaboom").unwrap_err();
    assert!(!bad.0.contains("kaboom"));
    assert!(!bad.0.is_empty());
    // Two limits or two --yes flags are usage errors, not silent overrides.
    assert!(parse_contract_command("approve-all", "10 20").is_err());
    assert!(parse_contract_command("approve-all", "--yes --yes").is_err());
}
