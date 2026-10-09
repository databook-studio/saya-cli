use super::*;

/// The resolved defaults convert into the run contract's `Budgets` shape —
/// the type the engine layers RunSpec and step budgets over — and the result
/// passes the contract's own validation.
#[test]
fn resolved_jobs_convert_into_contract_budgets_that_validate() {
    let resolved = resolve_with_user(
        "[jobs]\nturns = 40\ntool_calls = 25\nwall_clock_seconds = 1800\n\
         [jobs.tokens_per_endpoint]\n\"local-ollama\" = 200_000\n",
    );
    let budgets = resolved.jobs.budgets();
    budgets
        .validate()
        .expect("resolved jobs satisfy the run contract");
    assert_eq!(budgets.turns, Some(40));
    assert_eq!(budgets.tool_calls, Some(25));
    assert_eq!(budgets.wall_clock, Some(Duration::from_secs(1800)));
    assert_eq!(
        budgets.tokens_per_endpoint.get("local-ollama"),
        Some(&200_000)
    );

    // With nothing declared, the conversion carries no turn ceiling.
    let default_budgets = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("resolution succeeds")
        .jobs
        .budgets();
    default_budgets
        .validate()
        .expect("the default jobs satisfy the run contract");
    assert_eq!(default_budgets.turns, None);
}

/// `ConfigFile` is `deny_unknown_fields`, so an unknown key inside `[jobs]`
/// fails at parse time with the offending key named — a typo can never
/// silently fall back to a default.
#[test]
fn jobs_unknown_keys_are_rejected_naming_the_key() {
    let error = ConfigFile::from_toml("[jobs]\nfrobnicate = 1\n")
        .expect_err("an unknown [jobs] key must be rejected");
    let display = format!("{error}");
    assert!(
        display.contains("frobnicate"),
        "the parse error must name the unknown key, not just the section: {display}"
    );
}
