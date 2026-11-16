use super::*;

/// Diagnostics mirror the run ceilings in both views: the file view shows what
/// was declared (`None`/empty when unset), the resolved view shows the
/// effective values, and the JSON `config show` prints names them.
#[test]
fn diagnostics_report_the_run_limits_like_their_neighbours() {
    // File view: declared values are carried; unset ones are absent.
    let set = ConfigFile::from_toml("[limits.run]\nturns = 40\ntool_calls = 25\n").unwrap();
    let declared = set.redacted_diagnostics();
    assert_eq!(declared.limits_run_turns, Some(toml::Value::Integer(40)));
    assert_eq!(
        declared.limits_run_tool_calls,
        Some(toml::Value::Integer(25))
    );
    assert_eq!(declared.limits_run_wall_clock_seconds, None);
    assert_eq!(declared.limits_run_tokens_per_endpoint, None);
    let unset = ConfigFile::from_toml("[run]\nmax_rows = 10\n").unwrap();
    assert_eq!(unset.redacted_diagnostics().limits_run_turns, None);

    // Resolved view: the effective values; an undeclared ceiling is unlimited
    // (`None`), matching every other opt-in budget dimension.
    let resolved_set = resolve_with_user("[limits.run]\nturns = 30\n");
    let shown = resolved_set.redacted_diagnostics();
    assert_eq!(shown.limits_run_turns, Some(30));
    assert_eq!(shown.limits_run_tool_calls, None);
    assert_eq!(shown.limits_run_wall_clock_seconds, None);
    assert!(shown.limits_run_tokens_per_endpoint.is_empty());
    let rendered = serde_json::to_string(&shown).unwrap();
    assert!(
        rendered.contains("\"limits_run_turns\":30"),
        "resolved diagnostics must report limits_run_turns: {rendered}"
    );
    let unresolved = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("resolution succeeds")
        .redacted_diagnostics();
    assert_eq!(unresolved.limits_run_turns, None);

    // A token ceiling is echoed in both views without redaction: endpoint
    // names and token counts are user-declared figures, not secrets.
    let tokens = resolve_with_user("[limits.run.tokens_per_endpoint]\nollama = 200_000\n");
    let shown_tokens = tokens.redacted_diagnostics();
    assert_eq!(
        shown_tokens.limits_run_tokens_per_endpoint,
        BTreeMap::from([("ollama".into(), 200_000)])
    );
    let rendered = serde_json::to_string(&tokens.redacted_diagnostics()).unwrap();
    assert!(
        rendered.contains("\"limits_run_tokens_per_endpoint\":{\"ollama\":200000}"),
        "resolved diagnostics must report the token ceilings: {rendered}"
    );
}
