//! Unit tests for the MCP serve policy: the allowlist rule (invariant 2 —
//! explicit `--profile` values, else the configured default, else none), the
//! byte bounds, and the in-flight cap.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use super::policy::{MAX_IN_FLIGHT, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, ServePolicy};
use super::server::SayaServer;
use crate::config::runtime::RuntimeConfig;
use saya_config::{CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, resolve};

fn fixture_runtime(connections_toml: &str, config_toml: &str) -> RuntimeConfig {
    let connections = ConnectionsFile::from_toml(connections_toml).unwrap();
    let user = ConfigFile::from_toml(config_toml).unwrap();
    let resolved = resolve(
        ResolutionInput::new(connections.clone())
            .with_user(user)
            .with_cli(CliOverrides::default()),
    )
    .unwrap();
    RuntimeConfig {
        resolved,
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::new(),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: BTreeMap::new(),
    }
}

const TWO_PROFILES: &str = "[profiles.first]\ntype = 'sqlite'\npath = 'a.sqlite3'\n\n\
                            [profiles.second]\ntype = 'sqlite'\npath = 'b.sqlite3'\n";

/// Explicit `--profile` values win over the configured default, keep their
/// order, and deduplicate; with none, the default forms the allowlist; with
/// no default and no connections, the allowlist is empty and the server
/// still starts.
#[test]
fn the_allowlist_is_explicit_then_default_then_none() {
    let runtime = fixture_runtime(TWO_PROFILES, "default_profile = 'first'\n");
    let explicit = ServePolicy::resolve(&runtime, &["second".to_owned()], None).unwrap();
    assert_eq!(
        explicit.allowlist(),
        [second("second")].as_slice(),
        "an explicit --profile wins over the configured default"
    );
    let ordered = ServePolicy::resolve(
        &runtime,
        &["second".to_owned(), "first".to_owned(), "second".to_owned()],
        None,
    )
    .unwrap();
    assert_eq!(
        ordered.allowlist(),
        [second("second"), second("first")].as_slice(),
        "serve-local order is kept and duplicates collapse"
    );
    let default = ServePolicy::resolve(&runtime, &[], None).unwrap();
    assert_eq!(default.allowlist(), &[second("first")][..]);
    let pre_subcommand = ServePolicy::resolve(&runtime, &[], Some("second")).unwrap();
    assert_eq!(pre_subcommand.allowlist(), &[second("second")][..]);

    let bare = fixture_runtime("", "");
    let none = ServePolicy::resolve(&bare, &[], None).unwrap();
    assert!(
        none.allowlist().is_empty(),
        "no profile anywhere: the server starts with an empty allowlist"
    );
}

fn second(name: &str) -> super::policy::ProfileSummary {
    super::policy::ProfileSummary {
        name: name.to_owned(),
        dialect: "sqlite",
    }
}

/// A `--profile` naming nothing in the resolved connections is refused, not
/// silently served as an empty allowlist.
#[test]
fn an_unknown_profile_is_refused() {
    let runtime = fixture_runtime(TWO_PROFILES, "default_profile = 'first'\n");
    let error = ServePolicy::resolve(&runtime, &["missing".to_owned()], None)
        .expect_err("an unknown profile refuses");
    assert!(
        error.contains("missing"),
        "the refusal names the profile: {error}"
    );
}

/// The bounds are the constants the data tools (task Db) inherit: the byte
/// caps are inclusive, the call timeout is 30 s, and the in-flight cap is
/// enforced by slot acquisition and released on drop.
#[test]
fn the_bounds_hold_the_stated_values() {
    let runtime = fixture_runtime("", "");
    let policy = ServePolicy::resolve(&runtime, &[], None).unwrap();
    assert!(policy.request_allowed(MAX_REQUEST_BYTES));
    assert!(!policy.request_allowed(MAX_REQUEST_BYTES + 1));
    assert!(policy.response_allowed(MAX_RESPONSE_BYTES));
    assert!(!policy.response_allowed(MAX_RESPONSE_BYTES + 1));
    assert_eq!(policy.call_timeout(), Duration::from_secs(30));
    assert_eq!(policy.max_in_flight(), MAX_IN_FLIGHT);

    let server = SayaServer::new(policy);
    let mut slots = Vec::new();
    for _ in 0..MAX_IN_FLIGHT {
        slots.push(server.acquire_in_flight().expect("up to the cap"));
    }
    let refused = server.acquire_in_flight();
    assert!(
        refused.is_err() && refused.err().unwrap().message.contains("in-flight"),
        "the refusal says why"
    );
    drop(slots.pop());
    let _reacquired = server
        .acquire_in_flight()
        .expect("a dropped slot frees one");
}

/// `list_profiles` answers with names and dialects only — the shape the
/// stdio contract test pins over the wire, pinned here at the source.
#[test]
fn the_profiles_payload_names_dialects_only() {
    let runtime = fixture_runtime(
        "[profiles.local]\ntype = 'sqlite'\npath = 'secret/data.sqlite3'\n",
        "",
    );
    let policy = ServePolicy::resolve(&runtime, &["local".to_owned()], None).unwrap();
    assert_eq!(
        super::tools::profiles_payload(&policy),
        serde_json::json!({"profiles": [{"name": "local", "dialect": "sqlite"}]}),
    );
}
