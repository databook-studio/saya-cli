//! `[limits.database]` — what a turn's database work may spend, and the
//! one-release mapping of the retired `[limits.session]` and `[limits.ask]`
//! tables onto it.
//!
//! The old tables never bounded a conversation: they feed one turn's database
//! budget. A user who wrote one meant to bound that work, so its keys are read
//! rather than dropped — dropping them would silently widen a bound — and each
//! retired table raises exactly one diagnostic naming its replacement.

use saya_config::{
    BudgetLimit, CliOverrides, ConfigError, ConfigFile, ConnectionsFile, ResolutionInput,
    ResolvedAnsweringLimits, ResolvedConfig, resolve,
};

fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}

fn resolve_user(toml: &str) -> ResolvedConfig {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("user fixture parses")),
    )
    .expect("resolution succeeds")
}

fn resolve_layers(user: &str, project: &str, trust_project: bool) -> ResolvedConfig {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(user).expect("user fixture parses"))
            .with_project(ConfigFile::from_toml(project).expect("project fixture parses"))
            .with_cli(CliOverrides {
                trust_project_config: trust_project,
                ..Default::default()
            }),
    )
    .expect("resolution succeeds")
}

/// The sentence a renamed table raises, pinned verbatim: it is what a user
/// reads to learn where their ceiling went.
const RENAMED_SESSION: &str = "[limits.session] is now [limits.database]: it bounds \
                               database work in a turn, not the session; rename the table \
                               (accepted until the next release)";
const RENAMED_ASK: &str = "[limits.ask] is now [limits.database]: it bounds database work \
                           in a turn, not the session; rename the table (accepted until the \
                           next release)";

/// The new table states its four dimensions, and needs no diagnostic: it is
/// the spelling the release expects.
#[test]
fn limits_database_states_the_turn_budget() {
    let limits = resolve_user(
        "[limits.database]\nrequests = 5\ntool_calls = 9\nelapsed_seconds = 30\n\
         known_tokens = 1000\n",
    )
    .limits;
    assert_eq!(
        limits.database,
        ResolvedAnsweringLimits {
            requests: finite(5),
            tool_calls: finite(9),
            elapsed_seconds: finite(30),
            known_tokens: finite(1000),
        }
    );
    assert!(
        limits.diagnostics.is_empty(),
        "the current table raises no diagnostic: {:?}",
        limits.diagnostics
    );
}

/// The defaults are the numbers the retired table carried, so a config that
/// states none of them resolves exactly what it resolved before.
#[test]
fn the_database_defaults_are_the_retired_session_defaults() {
    let limits = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("default config resolves")
        .limits;
    assert_eq!(
        limits.database,
        ResolvedAnsweringLimits {
            requests: finite(24),
            tool_calls: finite(64),
            elapsed_seconds: finite(300),
            known_tokens: finite(250_000),
        }
    );
    assert!(limits.diagnostics.is_empty());
}

/// `[limits.session]` alone becomes the database budget, with exactly one
/// diagnostic carrying the exact sentence. The old field keeps its value for
/// the consumers that still read it.
#[test]
fn a_session_table_maps_onto_the_database_budget_with_one_diagnostic() {
    let limits = resolve_user("[limits.session]\nrequests = 5\n").limits;
    assert_eq!(limits.database.requests, finite(5));
    assert_eq!(
        limits.database.tool_calls,
        finite(64),
        "a key the old table leaves unset keeps the database default"
    );
    assert_eq!(
        limits.diagnostics,
        vec![RENAMED_SESSION.to_owned()],
        "one diagnostic, exactly"
    );
    assert_eq!(
        limits.session.requests,
        finite(5),
        "the old field still carries the session's own value"
    );
}

/// `[limits.ask]` is an `ask` override only: it bounds `saya ask` turns and
/// never the database budget, and it raises the same sentence naming itself.
#[test]
fn the_ask_table_maps_for_ask_only() {
    let limits = resolve_user("[limits.ask]\nrequests = 2\n").limits;
    assert_eq!(limits.ask_override.requests, finite(2));
    assert_eq!(
        limits.ask_override.elapsed_seconds,
        finite(300),
        "an unstated ask key keeps the database value"
    );
    assert_eq!(
        limits.database,
        ResolvedAnsweringLimits::default(),
        "ask bounds ask turns, not the database work"
    );
    assert_eq!(limits.diagnostics, vec![RENAMED_ASK.to_owned()]);
}

/// The ask override falls back to the database budget, so a key the ask table
/// leaves out is bounded by what the database table states.
#[test]
fn an_ask_key_falls_back_to_the_database_budget() {
    let limits = resolve_user(
        "[limits.database]\nrequests = 5\nelapsed_seconds = 30\n\
         [limits.ask]\nrequests = 2\n",
    )
    .limits;
    assert_eq!(limits.ask_override.requests, finite(2));
    assert_eq!(limits.ask_override.elapsed_seconds, finite(30));
}

/// Both tables stated: the new one wins, and the diagnostic says the retired
/// one was ignored rather than renamed.
#[test]
fn the_new_table_wins_over_a_retired_one() {
    let limits =
        resolve_user("[limits.session]\nrequests = 8\n[limits.database]\nrequests = 5\n").limits;
    assert_eq!(
        limits.database.requests,
        finite(5),
        "the new table states the budget"
    );
    assert_eq!(limits.diagnostics.len(), 1, "{:?}", limits.diagnostics);
    let message = &limits.diagnostics[0];
    for expected in ["[limits.session]", "[limits.database]", "ignored"] {
        assert!(
            message.contains(expected),
            "the diagnostic must say {expected}: {message}"
        );
    }
}

/// An invalid dimension is refused exactly as the session table's is, naming
/// the key the user wrote.
#[test]
fn a_database_dimension_is_validated_like_the_session_one() {
    for (value, key) in [
        ("0", "limits.database.requests"),
        ("-1", "limits.database.tool_calls"),
        ("'not-unlimited'", "limits.database.elapsed_seconds"),
        ("true", "limits.database.known_tokens"),
    ] {
        let (section, name) = key.rsplit_once('.').expect("key is dotted");
        let config = ConfigFile::from_toml(&format!("[{section}]\n{name} = {value}\n"))
            .expect("the raw value parses as TOML");
        let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
            .expect_err("an invalid dimension is refused");
        assert!(
            matches!(&error, ConfigError::InvalidLimit { key: field } if *field == key),
            "{key}: {error}"
        );
    }
}

/// The database budget is a security-critical ceiling like the tables it
/// replaces: a project layer cannot raise what the user set.
#[test]
fn a_project_layer_cannot_raise_the_database_budget() {
    let resolved = resolve_layers(
        "[limits.database]\nrequests = 5\n",
        "[limits.database]\nrequests = 'unlimited'\n",
        false,
    );
    assert_eq!(resolved.limits.database.requests, finite(5));
    assert_eq!(
        resolved.ignored_project_overrides,
        vec!["limits.database".to_owned()]
    );
}
