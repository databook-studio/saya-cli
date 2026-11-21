use super::*;

/// `/approve-all` — the deprecated spelling of `/contracts confirm-all` —
/// defaults to the deny-by-default preview (no `--yes`), `--yes` flips it, an
/// optional number overrides the limit, and anything else is a payload-free
/// usage error.
#[test]
fn parse_approve_all_translates_to_the_batch_command() {
    assert_eq!(
        command_of("/approve-all"),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: None,
            yes: false,
        }
    );
    assert_eq!(
        command_of("/approve-all --yes"),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: None,
            yes: true,
        }
    );
    assert_eq!(
        command_of("/approve-all --yes 10"),
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

/// The deprecated `/approve-all` and the canonical `/contracts confirm-all` are
/// one operation: every accepted argument shape parses to the same command.
#[test]
fn approve_all_and_contracts_confirm_all_are_one_operation() {
    for tail in ["", " --yes", " --yes 10", " 5"] {
        assert_eq!(
            command_of(&format!("/approve-all{tail}")),
            command_of(&format!("/contracts confirm-all{tail}")),
            "/approve-all and /contracts confirm-all disagree on {tail:?}"
        );
    }
    // The deprecated spelling states the note; the canonical one states none.
    let alias = parse_contract_command("approve-all", "--yes")
        .unwrap()
        .unwrap();
    let canonical = parse_contract_command("contracts", "confirm-all --yes")
        .unwrap()
        .unwrap();
    assert!(alias.note.is_some(), "/approve-all must say it is an alias");
    assert_eq!(canonical.note, None, "/contracts confirm-all is canonical");
}
