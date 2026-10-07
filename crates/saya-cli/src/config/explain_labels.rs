//! Display labels for the `config show --explain` rows: the dotted key each
//! row shows, kept separate from the key the origin is looked up under.

/// The dotted key an `--explain` row displays for one diagnostics field.
///
/// Most settings explain themselves: the display key is the dotted config
/// key the merge funnels record the origin under. The three `[jobs.runner]`
/// diagnostics fields share one grouped lookup key — `jobs.runner`, the
/// whole section as the resolver records it — so emitting the lookup key for
/// each would render three identical `jobs.runner` rows. Each of the three
/// gets a distinct display label instead, while the origin lookup keeps the
/// grouped key: the recorder records the section's origin as a whole, and
/// the display must not claim per-field precision the resolver never
/// recorded.
pub(crate) fn display_key(field: &str, lookup_key: &str) -> String {
    match field {
        "jobs_runner_allow" => "jobs.runner.allow".to_owned(),
        "jobs_runner_program_dir" => "jobs.runner.program_dir".to_owned(),
        "jobs_runner_timeout_seconds" => "jobs.runner.timeout_seconds".to_owned(),
        _ => lookup_key.to_owned(),
    }
}
