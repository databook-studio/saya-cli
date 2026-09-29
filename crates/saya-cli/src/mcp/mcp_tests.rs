//! Unit tests for the MCP serve policy (task Da + Db): the allowlist rule
//! (explicit `--profile` values, else the configured default, else none), the
//! byte bounds, the in-flight cap, the profiles payload shape, and the
//! catalog's data-sharing gate. The wire behavior is pinned in
//! `tests/mcp_stdio.rs` against the real binary.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use super::policy::{MAX_IN_FLIGHT, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, ServePolicy};
use super::server::SayaServer;
use crate::commands::{capture_output_start, capture_output_take};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
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

/// The replay's target resolution (F-1, D1): the effective target — the
/// `profile` argument, else the saved binding's profile — is resolved once,
/// inside the serialized replay section, and returned as the concrete
/// target the command will carry. A target outside the allowlist is refused
/// with a binding's name never echoed; a client-supplied name is refused
/// with the name it asked for; and with neither an argument nor a binding —
/// or a binding that cannot be read — the replay refuses `profile not
/// available`: there is no pass-through that lets the run command resolve
/// its own target at execution time (A922-1 — a binding read after a queued
/// wait can be remapped underneath the call).
#[test]
fn the_replay_resolution_binds_one_authorized_target() {
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
    use super::replay_tools::resolve_target;

    // A binding inside the allowlist resolves to the concrete target the
    // command will carry — not a pass-through verdict.
    let target = resolve_target(&policy, &context, local_bound.as_str(), None)
        .expect("a binding inside the allowlist resolves");
    assert_eq!(
        target, "first",
        "the resolution is the binding's profile, returned concretely"
    );
    let target = resolve_target(&policy, &context, local_bound.as_str(), Some("first"))
        .expect("an allowlisted argument resolves");
    assert_eq!(target, "first", "the argument is the target");

    // A binding pointing outside the allowlist: refused, the name not echoed.
    let refused = resolve_target(&policy, &context, other_bound.as_str(), None)
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
    let refused = resolve_target(&policy, &context, other_bound.as_str(), Some("second"))
        .expect_err("a non-allowlisted argument is refused");
    assert!(
        refused.contains("profile not available") && refused.contains("second"),
        "the requested name is echoed: {refused}"
    );

    // Neither an argument nor a binding — and a malformed id: the replay
    // refuses at the MCP layer. There is no pass-through (D1): a target the
    // MCP layer could not name is not something the run may resolve on its
    // own at execution time.
    let refused = resolve_target(&policy, &context, "no-such-id", None)
        .expect_err("a missing binding refuses");
    assert!(
        refused.contains("profile not available"),
        "the refusal names the gate: {refused}"
    );
    let refused = resolve_target(&policy, &context, "REFUSED-ID", None)
        .expect_err("a malformed id has no resolvable target");
    assert!(
        refused.contains("profile not available"),
        "the refusal names the gate: {refused}"
    );

    // A binding that exists but cannot be read fails the resolution CLOSED:
    // the replay does not start, and neither the store error's own text nor
    // the bound profile name is echoed.
    let corrupt = InvestigationId::parse("corrupt-binding-1").unwrap();
    let corrupt_path = runtime
        .investigations_root
        .join("local")
        .join(format!("{}.json", corrupt.as_str()));
    std::fs::write(&corrupt_path, b"not json at all").unwrap();
    let refused = resolve_target(&policy, &context, corrupt.as_str(), None)
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

/// The deterministic remap regression (A922-1, D1): the target resolved at
/// admission is the one the command carries — so when the ordinary CLI
/// remaps the investigation's binding to a non-allowlisted profile between
/// resolution and execution (the queued-call window), the run refuses as
/// stale and the other database's row never appears; and a resolution taken
/// after the remap refuses outright. The rebind is the same operation the
/// CLI drives (`--connection other --revalidate`), run in-process.
#[tokio::test]
async fn a_binding_remapped_under_a_queued_replay_never_runs() {
    const SENTINEL: &str = "OTHER-SYNTHETIC-SENTINEL-4";
    let root = std::env::temp_dir().join(format!("saya-mcp-remap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let local_db = root.join("data.sqlite3");
    let other_db = root.join("other.sqlite3");
    let mut runtime = fixture_runtime(
        &format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n\
             [profiles.other]\ntype = 'sqlite'\npath = '{}'\n",
            local_db.display(),
            other_db.display(),
        ),
        "default_profile = 'local'\n",
    );
    runtime.investigations_root = root.join("investigations");
    std::fs::create_dir_all(&runtime.investigations_root).unwrap();
    // Both databases carry an `events` table with the same schema; the
    // non-allowlisted one holds a row the allowlisted one does not, so a
    // wrongly-targeted replay is visible in the payload.
    seed_events(&local_db, "benign").await;
    seed_events(&other_db, SENTINEL).await;
    let repo = InvestigationRepository::new(runtime.investigations_root.clone());
    let state_db = saya_store::SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&runtime, &state_db).await;
    let parsed = InvestigationId::parse(&id).unwrap();

    let context = super::context::McpContext {
        runtime: runtime.clone(),
        store: saya_store::SqliteStateStore::new(root.join("state.sqlite3")),
        replay_slot: tokio::sync::Mutex::new(()),
    };
    let policy = ServePolicy::resolve(&runtime, &["local".to_owned()], None).unwrap();
    use super::replay_tools::resolve_target;

    // Admission: the binding names `local`, the only allowlisted profile —
    // the target resolves to the concrete name the command will carry.
    let target = resolve_target(&policy, &context, &id, None).expect("the binding is allowlisted");
    assert_eq!(target, "local");

    // The ordinary CLI remap: the same run operation the CLI drives,
    // `--connection other --revalidate`, succeeds and rewrites the binding
    // to the profile this server does not serve.
    capture_output_start();
    let rebind = crate::commands::run_investigation_outcome(
        crate::cli::InvestigationCommand::Run {
            id: id.clone(),
            connection: Some("other".to_owned()),
            revalidate: true,
            report: None,
            rows: None,
            overwrite: false,
            params: Vec::new(),
        },
        &runtime,
        RenderFormat::Text,
        false,
        &state_db,
    )
    .await
    .unwrap();
    let (out, err) = capture_output_take();
    assert_eq!(rebind.code, 0, "the CLI rebind succeeds: {out}{err}");
    let moved = repo.get_binding(&parsed).unwrap().expect("binding exists");
    assert_eq!(
        moved.profile, "other",
        "the rebind moved the binding off the allowlist"
    );

    // The queued call's resolution now happens (as the flow does, after the
    // wait): the binding names a profile outside the allowlist — refused
    // before anything runs, the name never echoed.
    let refused = resolve_target(&policy, &context, &id, None)
        .expect_err("the remapped binding refuses at resolution");
    assert!(
        refused.contains("profile not available"),
        "the refusal names the gate: {refused}"
    );
    assert!(
        !refused.contains("other") && !refused.contains(SENTINEL),
        "neither the profile nor the other database's data is echoed: {refused}"
    );

    // And when the resolution raced ahead of the remap, the command the
    // adapter builds carries the resolved target explicitly, so the re-read
    // binding is stale against it — refused, never executed.
    capture_output_start();
    let outcome = crate::commands::run_investigation_outcome(
        crate::cli::InvestigationCommand::Run {
            id: id.clone(),
            connection: Some(target),
            revalidate: false,
            report: None,
            rows: None,
            overwrite: false,
            params: Vec::new(),
        },
        &runtime,
        RenderFormat::Text,
        false,
        &state_db,
    )
    .await
    .unwrap();
    let (out, err) = capture_output_take();
    assert_ne!(
        outcome.code, 0,
        "the remapped binding never runs against the resolved target: {out}{err}"
    );
    assert!(outcome.replay.is_none(), "a refusal carries no replay");
    assert!(
        out.contains("target changed") || err.contains("target changed"),
        "the refusal is the target staleness: {out}{err}"
    );
    assert!(
        !out.contains(SENTINEL) && !err.contains(SENTINEL),
        "the other database's row never leaves its database: {out}{err}"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Seeds an `events` table with one labelled row into a sqlite file, the
/// way the binary-level fixtures do; only the test writes.
async fn seed_events(database: &std::path::Path, label: &str) {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO events (id, label) VALUES (1, ?)")
        .bind(label)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// Saves one investigation through the dispatcher (save never connects) and
/// returns its id, with the save's own rendering captured. The dispatcher
/// derives its repository from `runtime.investigations_root`, the same root
/// the test's own `repo` reads.
async fn saved_id(
    runtime: &crate::config::runtime::RuntimeConfig,
    state_db: &saya_store::SqliteStateStore,
) -> String {
    capture_output_start();
    let outcome = crate::commands::run_investigation_outcome(
        crate::cli::InvestigationCommand::Save {
            name: "remap probe".into(),
            description: None,
            sql: Some("SELECT label FROM events".into()),
            file: None,
            connection: Some("local".into()),
            param_specs: Vec::new(),
        },
        runtime,
        RenderFormat::Text,
        false,
        state_db,
    )
    .await
    .unwrap();
    let (out, err) = capture_output_take();
    assert_eq!(outcome.code, 0, "save failed: {out}{err}");
    out.lines()
        .next()
        .expect("save prints the id first")
        .trim()
        .to_string()
}
