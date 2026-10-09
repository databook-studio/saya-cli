use super::*;

/// `[jobs.fetch]` (M3-3) resolves to conservative download-budget defaults
/// when absent, and the declared keys resolve when present.
#[test]
fn jobs_fetch_resolves_defaults_and_declared_values() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    let fetch = &resolved.jobs.fetch;
    assert_eq!(
        *fetch,
        saya_config::ResolvedFetchJobs::default(),
        "the absent [jobs.fetch] resolves to the conservative defaults"
    );

    let declared = resolve_with_user(
        "[jobs.fetch]\nmax_file_bytes = 1024\nmax_run_bytes = 4096\ntimeout_seconds = 30\n",
    );
    assert_eq!(
        declared.jobs.fetch,
        saya_config::ResolvedFetchJobs {
            max_file_bytes: 1024,
            max_run_bytes: 4096,
            timeout_seconds: 30,
        }
    );
}

/// A below-minimum `[jobs.fetch]` value is a typed resolve error naming the
/// field — zero download bytes or a zero-second timeout would pause every
/// download before its first byte, a typo, not an intent. Never a silent
/// clamp.
#[test]
fn jobs_fetch_below_one_is_rejected_naming_the_field() {
    for (toml, field) in [
        ("[jobs.fetch]\nmax_file_bytes = 0\n", "fetch.max_file_bytes"),
        ("[jobs.fetch]\nmax_run_bytes = 0\n", "fetch.max_run_bytes"),
        (
            "[jobs.fetch]\ntimeout_seconds = 0\n",
            "fetch.timeout_seconds",
        ),
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
        )
        .unwrap_err();
        assert!(
            matches!(error, ConfigError::SettingBelowMinimum { min: 1, .. }),
            "expected SettingBelowMinimum for {toml:?}, got {error:?}"
        );
        let display = format!("{error}");
        assert!(
            display.contains(field),
            "error must name the field {field:?}: {display}"
        );
    }
}

/// An unknown key inside `[jobs.fetch]` is rejected at parse time with the
/// offending key named — `deny_unknown_fields` reaches the sub-table too.
#[test]
fn jobs_fetch_unknown_keys_are_rejected_naming_the_key() {
    let error = ConfigFile::from_toml("[jobs.fetch]\nfrobnicate = 1\n")
        .expect_err("an unknown [jobs.fetch] key must be rejected");
    let display = format!("{error}");
    assert!(
        display.contains("frobnicate"),
        "the parse error must name the unknown key: {display}"
    );
}
