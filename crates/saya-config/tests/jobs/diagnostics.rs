use super::*;

/// Diagnostics mirror the new settings in both views: the file view shows
/// what was declared (`None`/empty when unset), the resolved view shows the
/// effective values, and the JSON `config show` prints names them.
#[test]
fn diagnostics_report_jobs_like_their_neighbours() {
    // File view: declared values are carried; unset ones are absent.
    let set = ConfigFile::from_toml("[jobs]\nturns = 40\ntool_calls = 25\n").unwrap();
    let declared = set.redacted_diagnostics();
    assert_eq!(declared.jobs_turns, Some(40));
    assert_eq!(declared.jobs_tool_calls, Some(25));
    assert_eq!(declared.jobs_wall_clock_seconds, None);
    assert_eq!(declared.jobs_tokens_per_endpoint, None);
    let unset = ConfigFile::from_toml("[run]\nmax_rows = 10\n").unwrap();
    assert_eq!(unset.redacted_diagnostics().jobs_turns, None);

    // Resolved view: the effective values; an undeclared turn ceiling is
    // unlimited (`None`), matching every other opt-in budget dimension.
    let resolved_set = resolve_with_user("[jobs]\nturns = 30\n");
    let shown = resolved_set.redacted_diagnostics();
    assert_eq!(shown.jobs_turns, Some(30));
    assert_eq!(shown.jobs_tool_calls, None);
    assert_eq!(shown.jobs_wall_clock_seconds, None);
    assert!(shown.jobs_tokens_per_endpoint.is_empty());
    let rendered = serde_json::to_string(&shown).unwrap();
    assert!(
        rendered.contains("\"jobs_turns\":30"),
        "resolved diagnostics must report jobs_turns: {rendered}"
    );
    let unresolved = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("resolution succeeds")
        .redacted_diagnostics();
    assert_eq!(unresolved.jobs_turns, None);

    // A token ceiling is echoed in both views without redaction: endpoint
    // names and token counts are user-declared figures, not secrets.
    let tokens = resolve_with_user("[jobs.tokens_per_endpoint]\nollama = 200_000\n");
    let shown_tokens = tokens.redacted_diagnostics();
    assert_eq!(
        shown_tokens.jobs_tokens_per_endpoint,
        BTreeMap::from([("ollama".into(), 200_000)])
    );
    let rendered = serde_json::to_string(&tokens.redacted_diagnostics()).unwrap();
    assert!(
        rendered.contains("\"jobs_tokens_per_endpoint\":{\"ollama\":200000}"),
        "resolved diagnostics must report the token ceilings: {rendered}"
    );
}

/// The `[jobs.runner]` settings mirror beside their neighbours in both
/// views: the file view shows what was declared (`None` when unset), the
/// resolved view shows the effective values, and the JSON `config show`
/// names the fields — the path itself is user-declared, not a secret.
#[test]
fn diagnostics_report_the_runner_settings_like_their_neighbours() {
    let unset = ConfigFile::from_toml("[run]\nmax_rows = 10\n").unwrap();
    let declared_unset = unset.redacted_diagnostics();
    assert_eq!(declared_unset.jobs_runner_allow, None);
    assert_eq!(declared_unset.jobs_runner_program_dir, None);
    assert_eq!(declared_unset.jobs_runner_timeout_seconds, None);

    let directory_toml = toml_string(runner_program_dir());
    let set = ConfigFile::from_toml(&format!(
        "[jobs.runner]\nallow = [\"bench\"]\nprogram_dir = {directory_toml}\ntimeout_seconds = 45\n"
    ))
    .unwrap();
    let declared = set.redacted_diagnostics();
    assert_eq!(declared.jobs_runner_allow, Some(vec!["bench".to_owned()]));
    assert_eq!(
        declared.jobs_runner_program_dir,
        Some(runner_program_dir().to_owned())
    );
    assert_eq!(declared.jobs_runner_timeout_seconds, Some(45));

    let shown = resolve_with_user(&format!(
        "[jobs.runner]\nallow = [\"bench\"]\nprogram_dir = {}\n",
        directory_toml
    ))
    .redacted_diagnostics();
    assert_eq!(shown.jobs_runner_allow, vec!["bench".to_owned()]);
    assert_eq!(
        shown.jobs_runner_program_dir,
        Some(runner_program_dir().to_owned())
    );
    assert_eq!(shown.jobs_runner_timeout_seconds, 300);
    let rendered = serde_json::to_string(&shown).unwrap();
    assert!(
        rendered.contains(&format!(
            "\"jobs_runner_program_dir\":{}",
            serde_json::to_string(runner_program_dir()).unwrap()
        )),
        "resolved diagnostics must report the program directory: {rendered}"
    );
}
