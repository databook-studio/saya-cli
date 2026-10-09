use super::*;

/// `/use` is deliberately absent. `use_candidate_once` validates a claim but
/// the interactive session does not yet thread the admission into the next
/// recall, so the command could not do what its name promises. Shipping it
/// would have told a user their candidate was admitted when nothing had
/// changed — the exact overstatement this feature exists to avoid. It
/// returns here once the admission is threaded.
#[test]
fn use_is_not_a_contract_command_until_the_admission_is_threaded() {
    assert!(
        parse_contract_command("use", "c-1").unwrap().is_none(),
        "/use must not parse while it cannot admit anything"
    );
}

#[test]
fn parse_unknown_name_returns_none() {
    assert!(parse_contract_command("sql", "select 1").unwrap().is_none());
}
