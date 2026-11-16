use super::*;

/// The four `[jobs]` ceilings are refused: a run's ceilings are stated in
/// `[limits.run]`, one vocabulary with every other surface's. Each retired key
/// names the line to write instead — a moved setting is not a typo.
#[test]
fn the_jobs_ceiling_keys_name_their_limits_run_replacement() {
    for (key, replacement) in [
        ("wall_clock_seconds", "limits.run.wall_clock_seconds"),
        ("tokens_per_endpoint", "limits.run.tokens_per_endpoint"),
        ("turns", "limits.run.turns"),
        ("tool_calls", "limits.run.tool_calls"),
    ] {
        let toml = if key == "tokens_per_endpoint" {
            format!("[jobs.{key}]\nollama = 1000\n")
        } else {
            format!("[jobs]\n{key} = 5\n")
        };
        let error = ConfigFile::from_toml(&toml).expect_err("a retired key is refused");
        let message = error.to_string();
        assert!(
            message.contains(&format!("was moved: write {replacement} instead")),
            "{key}: {message}"
        );
    }
}

/// `[jobs.fetch]` did not move: it is a download budget with concrete
/// defaults, not a run ceiling, so it keeps its section and its keys.
#[test]
fn jobs_fetch_is_not_retired_and_still_parses_where_it_is() {
    let config = ConfigFile::from_toml("[jobs.fetch]\nmax_file_bytes = 1024\n")
        .expect("[jobs.fetch] must still parse");
    assert_eq!(
        config.jobs.fetch.and_then(|fetch| fetch.max_file_bytes),
        Some(1024)
    );
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
