//! Unit tests for the MCP serve policy (task Da + Db): the allowlist rule
//! (explicit `--profile` values, else the configured default, else none), the
//! byte bounds, the in-flight cap, the profiles payload shape, and the
//! catalog's data-sharing gate. The wire behavior is pinned in
//! `tests/mcp_stdio.rs` against the real binary.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use super::policy::{MAX_IN_FLIGHT, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, ServePolicy};
use super::server::SayaServer;
use crate::config::runtime::RuntimeConfig;
use saya_config::{CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, resolve};
use saya_store::{InvestigationRepository, LocalBinding};
use saya_types::InvestigationId;

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

    let server = SayaServer::new(
        policy,
        std::sync::Arc::new(super::context::McpContext {
            runtime: fixture_runtime("", ""),
            store: saya_store::SqliteStateStore::new(
                std::env::temp_dir().join("saya-mcp-test-state"),
            ),
            replay_slot: tokio::sync::Mutex::new(()),
        }),
    );
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

/// The catalog gates the row-returning tools on data sharing: with sharing
/// off they are absent from `tools/list` entirely; with it on, the whole
/// toolset is advertised. The dispatch-side refusal for a row tool called
/// anyway is pinned over the wire in `tests/mcp_stdio.rs`
/// (`mcp_denial_matches_cli_policy`).
#[test]
fn the_catalog_gates_row_tools_on_data_sharing() {
    let runtime = fixture_runtime(TWO_PROFILES, "default_profile = 'first'\n");
    let sharing_on = ServePolicy::resolve(&runtime, &["first".to_owned()], None)
        .unwrap()
        .with_data_sharing(true);
    let sharing_off = ServePolicy::resolve(&runtime, &["first".to_owned()], None)
        .unwrap()
        .with_data_sharing(false);
    let listed_on: Vec<String> = super::catalog::advertised(&sharing_on)
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    let listed_off: Vec<String> = super::catalog::advertised(&sharing_off)
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    assert_eq!(
        listed_on,
        [
            "contracts",
            "investigation_run",
            "list_profiles",
            "query",
            "schema"
        ],
    );
    assert_eq!(listed_off, ["contracts", "list_profiles", "schema"]);
    assert!(super::catalog::is_row_returning("query"));
    assert!(super::catalog::is_row_returning("investigation_run"));
    assert!(!super::catalog::is_row_returning("schema"));
    assert!(!super::catalog::is_row_returning("list_profiles"));
}

/// The replay tool's `params` argument reads as an ordered `name=value`
/// list for the run command: absent and empty collapse to none, every value
/// must be a string, and a non-object map is a parameter error.
#[test]
fn the_params_argument_reads_as_a_name_value_list() {
    fn args(arguments: serde_json::Value) -> rmcp::model::CallToolRequestParams {
        serde_json::from_value(
            serde_json::json!({"name": "investigation_run", "arguments": arguments}),
        )
        .unwrap()
    }
    let none = super::tools::optional_string_map(&args(serde_json::json!({})), "params").unwrap();
    assert!(
        none.is_empty(),
        "no params argument binds nothing: {none:?}"
    );

    let empty =
        super::tools::optional_string_map(&args(serde_json::json!({"params": {}})), "params")
            .unwrap();
    assert!(empty.is_empty(), "an empty map binds nothing: {empty:?}");

    let bound = super::tools::optional_string_map(
        &args(serde_json::json!({"params": {"since": "2024-01-01", "label": "a=b", "count": "3"}})),
        "params",
    )
    .unwrap();
    let mut sorted = bound.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        ["count=3", "label=a=b", "since=2024-01-01"],
        "each entry is one canonical name=value string: {bound:?}"
    );

    let not_object =
        super::tools::optional_string_map(&args(serde_json::json!({"params": ["x"]})), "params");
    assert!(
        not_object.is_err() && not_object.err().unwrap().message.contains("object"),
        "a non-object params is a parameter error"
    );

    let not_string = super::tools::optional_string_map(
        &args(serde_json::json!({"params": {"count": 3}})),
        "params",
    )
    .expect_err("a non-string value is refused");
    assert!(
        not_string.message.contains("count") && not_string.message.contains("string"),
        "a non-string value is a parameter error naming the key: {}",
        not_string.message
    );
}

/// The replay's allowlist gate (F-1): the effective target — the `profile`
/// argument, else the saved binding's profile, the run command's own
/// resolution — must sit inside the startup allowlist before anything runs.
/// A binding saved against a profile the server does not serve is refused
/// with its name never echoed; a client-supplied name is refused with the
/// name it asked for; and with neither an argument nor a readable binding
/// the gate passes, leaving the run command's own refusals in charge.
#[test]
fn the_replay_gate_holds_the_run_inside_the_allowlist() {
    fn binding(id: &InvestigationId, profile: &str) -> LocalBinding {
        LocalBinding {
            version: LocalBinding::VERSION,
            id: id.clone(),
            profile: profile.to_owned(),
            profile_identity: "identity".to_owned(),
            reviewed_revision: 1,
            reviewed_schema_fingerprint: None,
            reviewed_unix_ms: 0,
        }
    }

    let root = std::env::temp_dir().join(format!("saya-mcp-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut runtime = fixture_runtime(TWO_PROFILES, "default_profile = 'first'\n");
    runtime.investigations_root = root.clone();
    let repo = InvestigationRepository::new(root);
    let other_bound = InvestigationId::parse("bound-second-1").unwrap();
    let local_bound = InvestigationId::parse("bound-first-1").unwrap();
    repo.put_binding(&binding(&other_bound, "second")).unwrap();
    repo.put_binding(&binding(&local_bound, "first")).unwrap();

    let context = super::context::McpContext {
        runtime: runtime.clone(),
        store: saya_store::SqliteStateStore::new(std::env::temp_dir().join("saya-mcp-test-state")),
        replay_slot: tokio::sync::Mutex::new(()),
    };
    let policy = ServePolicy::resolve(&runtime, &["first".to_owned()], None).unwrap();
    use super::replay_tools::allowlist_gate;

    // A binding pointing outside the allowlist: refused, the name not echoed.
    let refused = allowlist_gate(&policy, &context, other_bound.as_str(), None)
        .expect_err("a binding outside the allowlist is refused");
    assert!(
        refused.contains("profile not available"),
        "the refusal names the gate: {refused}"
    );
    assert!(
        !refused.contains("second"),
        "the binding's profile is never echoed: {refused}"
    );

    // An argument naming the same profile: refused with the requested name.
    let refused = allowlist_gate(&policy, &context, other_bound.as_str(), Some("second"))
        .expect_err("a non-allowlisted argument is refused");
    assert!(
        refused.contains("profile not available") && refused.contains("second"),
        "the requested name is echoed: {refused}"
    );

    // Allowlisted on both shapes: the run may proceed.
    assert!(
        allowlist_gate(&policy, &context, local_bound.as_str(), None).is_ok(),
        "a binding inside the allowlist runs"
    );
    assert!(
        allowlist_gate(&policy, &context, local_bound.as_str(), Some("first")).is_ok(),
        "an allowlisted argument runs"
    );

    // Neither an argument nor a binding — and a malformed id: the gate
    // passes, leaving the run command's own refusals in charge (the same
    // words the CLI prints, in the CLI's own order).
    assert!(
        allowlist_gate(&policy, &context, "no-such-id", None).is_ok(),
        "an absent binding leaves the refusal to the run command"
    );
    assert!(
        allowlist_gate(&policy, &context, "REFUSED-ID", None).is_ok(),
        "a malformed id leaves the refusal to the run command"
    );

    // A binding that exists but cannot be read fails the gate CLOSED: the
    // run does not start, and neither the store error's own text nor the
    // bound profile name is echoed.
    let corrupt = InvestigationId::parse("corrupt-binding-1").unwrap();
    let corrupt_path = runtime
        .investigations_root
        .join("local")
        .join(format!("{}.json", corrupt.as_str()));
    std::fs::write(&corrupt_path, b"not json at all").unwrap();
    let refused = allowlist_gate(&policy, &context, corrupt.as_str(), None)
        .expect_err("an unreadable binding is refused");
    assert!(
        refused.contains("profile not available"),
        "the refusal names the gate: {refused}"
    );
    assert!(
        !refused.contains("not valid for storage") && !refused.contains("second"),
        "neither the store error nor a profile name is echoed: {refused}"
    );
}
