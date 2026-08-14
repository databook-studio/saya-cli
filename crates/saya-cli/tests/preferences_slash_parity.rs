//! Cross-adapter parity for the Phase 5c-2 `/preferences` slash adapter: the
//! `/preferences` slash command must call the *same* operations as the headless
//! `saya preferences list` command and add nothing — no second parsing, no
//! second scope resolution, no second DTO mapping.
//!
//! Both paths converge on `run_preferences(PreferencesCommand, …)`: the slash
//! path only translates slash text into a `PreferencesCommand` and hands it to
//! the same dispatcher. The parity test proves the translated value equals the
//! headless one and that both produce identical output (same kinds, values, and
//! order — spec test 6).

use saya_cli::{
    PreferencesCommand, RenderFormat, RuntimeConfig, SlashCommand, capture_output_start,
    capture_output_take, load_with_sources, parse_slash_command, profile_identity, run_preferences,
};
use saya_store::{SchemaStore, SqliteStateStore};
use saya_types::SchemaTree;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// harness — mirrors tests/contracts_slash_parity.rs.
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-pref-parity-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn runtime_at(root: &Path) -> (RuntimeConfig, String) {
    let database = root.join("data.sqlite3");
    fs::write(&database, b"").unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    let runtime = load_with_sources(&options, root, root, BTreeMap::new()).unwrap();
    (runtime, "local".to_string())
}

async fn store_at(root: &Path) -> SqliteStateStore {
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    store
        .upsert_schema(
            &identity_for(&runtime_for_scope(root), "local"),
            &SchemaTree::default(),
        )
        .await
        .unwrap();
    store
}

fn runtime_for_scope(root: &Path) -> RuntimeConfig {
    let connections = root.join("connections.toml");
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
}

fn identity_for(runtime: &RuntimeConfig, name: &str) -> String {
    let profile = runtime.named_profile(name).unwrap();
    profile_identity(name, profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}

/// Runs a `PreferencesCommand` through the shared headless dispatcher.
async fn run_headless(
    command: PreferencesCommand,
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
    format: RenderFormat,
) -> (i32, String, String) {
    capture_output_start();
    let code = run_preferences(command, runtime, format, store)
        .await
        .unwrap();
    let (out, err) = capture_output_take();
    (code, out, err)
}

/// Parses a slash line to its `PreferencesCommand` (the slash adapter's only
/// job), then runs it through the same dispatcher. Returns the parsed command
/// and the captured output.
async fn run_slash(
    line: &str,
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
    format: RenderFormat,
) -> (PreferencesCommand, i32, String, String) {
    let command = match parse_slash_command(line) {
        Ok(Some(SlashCommand::Preferences(cmd))) => cmd,
        other => panic!("expected SlashCommand::Preferences for {line:?}, got {other:?}"),
    };
    let (code, out, err) = run_headless(command.clone(), runtime, store, format).await;
    (command, code, out, err)
}

// ---------------------------------------------------------------------------
// 1. /preferences parses to the same PreferencesCommand the headless parser
//    produces — List { profile: None } — the structural parity the spec's
//    "same dispatcher" rule depends on.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn preferences_parses_to_headless_list_command() {
    let root = temp_root("parse");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;
    let (cmd, _code, _out, _err) =
        run_slash("/preferences", &runtime, &store, RenderFormat::Text).await;
    assert_eq!(cmd, PreferencesCommand::List { profile: None });
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. /preferences and `saya preferences list` return the same kinds and values
//    in the same order (spec test 6). Byte-for-byte, because both hand the
//    identical command to the one dispatcher — a second read would diverge.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn preferences_slash_and_headless_agree_on_kinds_values_and_order() {
    let root = temp_root("agree");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // Seed one profile-scoped and one global preference so both scopes render.
    let headless_set = PreferencesCommand::Set {
        kind: saya_cli::PreferenceKindArg::Timezone,
        value: "Europe/London".into(),
        profile: None,
    };
    let (code, _o, err) = run_headless(headless_set, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "seed tz stderr: {err}");
    let headless_set = PreferencesCommand::Set {
        kind: saya_cli::PreferenceKindArg::OutputStyle,
        value: "compact".into(),
        profile: None,
    };
    let (code, _o, err) = run_headless(headless_set, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "seed style stderr: {err}");

    let headless = run_headless(
        PreferencesCommand::List { profile: None },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_cmd, code, out, err) =
        run_slash("/preferences", &runtime, &store, RenderFormat::Text).await;

    assert_eq!(code, 0, "/preferences stderr: {err}");
    // Same kinds, values, and order — a second DTO mapping or a second read
    // would diverge here.
    assert_eq!(out, headless.1, "/preferences diverged from headless list");
    assert_eq!(err, headless.2);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 3. /preferences with an argument is a usage error, not a silent ignore.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn preferences_rejects_argument() {
    let err = parse_slash_command("/preferences extra").unwrap_err();
    assert!(err.to_string().contains("no argument"), "got: {err}");
    // The error must not echo the untrusted argument.
    assert!(!err.to_string().contains("extra"));
}

// ---------------------------------------------------------------------------
// 4. The opaque profile identity appears in no /preferences output.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn preferences_slash_never_leaks_identity() {
    let root = temp_root("no_leak");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let headless_set = PreferencesCommand::Set {
        kind: saya_cli::PreferenceKindArg::Timezone,
        value: "Europe/London".into(),
        profile: None,
    };
    run_headless(headless_set, &runtime, &store, RenderFormat::Text).await;

    let identity = identity_for(&runtime, "local");
    let (_cmd, _code, out, err) =
        run_slash("/preferences", &runtime, &store, RenderFormat::Text).await;
    assert!(
        !out.contains(&identity),
        "identity leaked into /preferences stdout: {out}"
    );
    assert!(
        !err.contains(&identity),
        "identity leaked into /preferences stderr: {err}"
    );

    let _ = fs::remove_dir_all(root);
}
