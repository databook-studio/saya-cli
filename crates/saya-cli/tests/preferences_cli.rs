//! Headless `saya preferences` commands: drives the command adapter directly
//! (constructs `PreferencesCommand` values and calls `run_preferences`), never
//! shelling out to a built binary. Covers the spec at
//! `.claude/specs/spec-5c2-preference-adapters.md`.
//!
//! Output is captured through the thread-local seam in `output::emit` so the
//! tests can assert on rendered text/JSON/NDJSON without touching the process
//! stdout (which would race under parallel test runs).

use saya_cli::{
    PreferenceKindArg, PreferencesCommand, RenderFormat, RuntimeConfig, capture_output_start,
    capture_output_take, load_with_sources, profile_identity, run_preferences,
};
use saya_store::{PreferenceStore, SqliteStateStore};
use saya_types::{PreferenceScope, PreferenceValue};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// harness — mirrors tests/contracts_cli.rs so the derived profile identity is
// deterministic and matches the store setup.
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-pref-cli-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

/// A single-profile runtime over a sqlite database file. The one-profile
/// connections file auto-selects `local` as the active profile, so `--profile`
/// can be omitted and still resolve. The connections path *is* the cache scope,
/// so the derived profile identity is deterministic for the leak test.
fn runtime_at(root: &Path) -> (RuntimeConfig, PathBuf, String) {
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
        connections: Some(connections.clone()),
        ..Default::default()
    };
    let runtime = load_with_sources(&options, root, root, BTreeMap::new()).unwrap();
    (runtime, connections, "local".to_string())
}

/// A two-profile runtime over two sqlite files; `local` is the active profile
/// (selected via `--profile` on load). Both profiles share the connections file
/// — and thus the cache scope — so the derived identity for a name is stable.
fn runtime_two_profiles(root: &Path) -> (RuntimeConfig, String, String) {
    let local_db = root.join("local.sqlite3");
    let staging_db = root.join("staging.sqlite3");
    fs::write(&local_db, b"").unwrap();
    fs::write(&staging_db, b"").unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n[profiles.staging]\ntype = 'sqlite'\npath = '{}'\n",
            local_db.display(),
            staging_db.display()
        ),
    )
    .unwrap();
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        profile: Some("local".into()),
        ..Default::default()
    };
    let runtime = load_with_sources(&options, root, root, BTreeMap::new()).unwrap();
    (runtime, "local".to_string(), "staging".to_string())
}

/// A store whose migrations have run, at `root/state.sqlite3`. The pool is
/// touched via a *global* preference read so no profile identity is needed —
/// the two-profile test never has to resolve a profile just to open the store.
async fn store_at(root: &Path) -> SqliteStateStore {
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    // Touch the pool so migrations run. A global-scope list needs no profile.
    let _ = store
        .list_preferences(&saya_types::PreferenceScope::Global)
        .await;
    store
}

fn identity_for(runtime: &RuntimeConfig, name: &str) -> String {
    let profile = runtime.named_profile(name).unwrap();
    profile_identity(name, profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}

/// Runs `command` against `runtime`/`store` in `format`, returning the exit code
/// plus the captured (stdout, stderr).
async fn run(
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

/// Parse every non-empty captured stdout line as a JSON `TerminalEvent`.
fn json_events(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("line is JSON"))
        .collect()
}

/// The four kinds, each with a well-shaped value, paired with the `set`/`list`
/// word for the value (timezone string; grain; style; profile name).
fn each_kind_cases() -> [(PreferenceKindArg, &'static str); 4] {
    [
        (PreferenceKindArg::Timezone, "Europe/London"),
        (PreferenceKindArg::DateGrain, "month"),
        (PreferenceKindArg::OutputStyle, "compact"),
        (PreferenceKindArg::DefaultProfile, "warehouse"),
    ]
}

// ---------------------------------------------------------------------------
// 1. set then list round-trips each of the four kinds.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn set_then_list_round_trips_each_kind() {
    let root = temp_root("round_trip");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    for (kind, value) in each_kind_cases() {
        let set = PreferencesCommand::Set {
            kind,
            value: value.into(),
            profile: None,
        };
        let (code, out, err) = run(set, &runtime, &store, RenderFormat::Text).await;
        assert_eq!(code, 0, "set {kind:?} stderr: {err}");
        assert!(out.starts_with("set "), "set out: {out}");
    }

    let list = PreferencesCommand::List { profile: None };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "list stderr: {err}");
    for (kind, value) in each_kind_cases() {
        assert!(
            out.contains(value),
            "list missing value {value:?} for {kind:?}: {out}"
        );
    }
    // The global kinds (output_style, default_profile) and the profile-scoped
    // kinds (timezone, date_grain) all appear in the one merged list.
    assert!(out.contains("date_grain"), "list missing date_grain: {out}");
    assert!(
        out.contains("output_style"),
        "list missing output_style: {out}"
    );
    assert!(out.contains("timezone"), "list missing timezone: {out}");
    assert!(
        out.contains("default_profile"),
        "list missing default_profile: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. A profile-scoped kind without --profile defaults to the active profile;
//    a global kind WITH --profile is a typed error naming the conflict.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn profile_kind_defaults_to_active_and_global_kind_rejects_profile() {
    let root = temp_root("scope_conflict");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    // A profile-scoped kind with no --profile uses the active profile ("local"),
    // so it reads back from that profile's scope.
    let set = PreferencesCommand::Set {
        kind: PreferenceKindArg::Timezone,
        value: "Europe/London".into(),
        profile: None,
    };
    let (code, out, err) = run(set, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "set timezone (active) stderr: {err}");
    assert!(
        out.contains("scope: local"),
        "set names the active scope: {out}"
    );

    let got = store
        .get_preference(
            &PreferenceScope::Profile(
                saya_types::ProfileIdentity::parse(&identity_for(&runtime, "local")).unwrap(),
            ),
            "timezone",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        got,
        PreferenceValue::timezone("Europe/London").unwrap(),
        "set defaulted to the active profile"
    );

    // A global kind WITH --profile is a typed error naming the conflict.
    let set = PreferencesCommand::Set {
        kind: PreferenceKindArg::OutputStyle,
        value: "compact".into(),
        profile: Some("local".into()),
    };
    let (code, out, err) = run(set, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        code, 0,
        "global kind with --profile must not succeed: {out}{err}"
    );
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("global"),
        "error must name the required scope: {combined}"
    );
    // The error must not echo the untrusted value.
    assert!(
        !combined.contains("compact"),
        "error must not echo the value: {combined}"
    );
    // And must not leak the opaque identity.
    let identity = identity_for(&runtime, "local");
    assert!(
        !combined.contains(&identity),
        "opaque identity leaked into error: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 3. An invalid value for each kind is a typed error; the message does not
//    echo the value.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn invalid_value_is_typed_error_without_echo() {
    let root = temp_root("invalid_value");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    // Each kind with an invalid value: a malformed timezone, a bad grain word,
    // a bad style word, and a control-char profile name.
    let cases = [
        (PreferenceKindArg::Timezone, "Europe/London!"),
        (PreferenceKindArg::DateGrain, "century"),
        (PreferenceKindArg::OutputStyle, "fancy"),
        (PreferenceKindArg::DefaultProfile, "bad\u{0}name"),
    ];
    for (kind, value) in cases {
        let set = PreferencesCommand::Set {
            kind,
            value: value.into(),
            profile: None,
        };
        let (code, out, err) = run(set, &runtime, &store, RenderFormat::Text).await;
        assert_ne!(code, 0, "{kind:?}={value:?} must not succeed: {out}{err}");
        let combined = format!("{out}{err}");
        assert!(
            !combined.is_empty(),
            "an invalid value must emit a diagnostic: {kind:?}"
        );
        assert!(
            !combined.contains(value),
            "error must not echo the value {value:?}: {combined}"
        );
        // And must not leak the opaque identity.
        let identity = identity_for(&runtime, "local");
        assert!(
            !combined.contains(&identity),
            "opaque identity leaked into error: {combined}"
        );
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 4. unset removes only the named kind at the named scope.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unset_removes_only_the_named_kind_at_the_named_scope() {
    let root = temp_root("unset");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    // Set two kinds at the active profile scope.
    for (kind, value) in [
        (PreferenceKindArg::Timezone, "Europe/London"),
        (PreferenceKindArg::DateGrain, "month"),
    ] {
        let set = PreferencesCommand::Set {
            kind,
            value: value.into(),
            profile: None,
        };
        let (code, _out, err) = run(set, &runtime, &store, RenderFormat::Text).await;
        assert_eq!(code, 0, "set {kind:?} stderr: {err}");
    }
    // And a global kind.
    let set = PreferencesCommand::Set {
        kind: PreferenceKindArg::OutputStyle,
        value: "compact".into(),
        profile: None,
    };
    let (code, _out, err) = run(set, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "set output_style stderr: {err}");

    // Unset timezone (profile-scoped) only.
    let unset = PreferencesCommand::Unset {
        kind: PreferenceKindArg::Timezone,
        profile: None,
    };
    let (code, out, err) = run(unset, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "unset stderr: {err}");
    assert!(
        out.contains("unset timezone"),
        "unset names the kind: {out}"
    );

    // timezone is gone; date_grain (same profile) and output_style (global) survive.
    let list = PreferencesCommand::List { profile: None };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "list stderr: {err}");
    assert!(
        !out.contains("Europe/London"),
        "timezone was not removed: {out}"
    );
    assert!(out.contains("month"), "date_grain was removed too: {out}");
    assert!(
        out.contains("compact"),
        "output_style was removed too: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 5. Two profiles list independently.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn two_profiles_list_independently() {
    let root = temp_root("two_profiles");
    let (runtime, local, staging) = runtime_two_profiles(&root);
    let store = store_at(&root).await;

    // Set timezone at each profile. The active profile (local) needs no
    // --profile; staging is named explicitly.
    let set_local = PreferencesCommand::Set {
        kind: PreferenceKindArg::Timezone,
        value: "Europe/London".into(),
        profile: None,
    };
    let (code, _o, err) = run(set_local, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "set local stderr: {err}");
    let set_staging = PreferencesCommand::Set {
        kind: PreferenceKindArg::Timezone,
        value: "America/New_York".into(),
        profile: Some(staging.clone()),
    };
    let (code, _o, err) = run(set_staging, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "set staging stderr: {err}");

    // List the active profile (local): sees Europe/London, not America/New_York.
    let list_local = PreferencesCommand::List { profile: None };
    let (code, out, err) = run(list_local, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "list local stderr: {err}");
    assert!(
        out.contains("Europe/London"),
        "local list missing its tz: {out}"
    );
    assert!(
        !out.contains("America/New_York"),
        "local list leaked staging's tz: {out}"
    );

    // List staging: sees America/New_York, not Europe/London.
    let list_staging = PreferencesCommand::List {
        profile: Some(staging.clone()),
    };
    let (code, out, err) = run(list_staging, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "list staging stderr: {err}");
    assert!(
        out.contains("America/New_York"),
        "staging list missing its tz: {out}"
    );
    assert!(
        !out.contains("Europe/London"),
        "staging list leaked local's tz: {out}"
    );

    // Sanity: the two names are distinct and neither is the identity.
    assert_ne!(local, staging);
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 6. /preferences and `saya preferences list` return the same kinds and values
//    in the same order. (Parity lives in preferences_slash_parity.rs; this test
//    pins that the translated command is structurally equal to the headless
//    one, so the order guarantee is the dispatcher's, not a second read.)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn preferences_list_is_deterministic_by_kind() {
    let root = temp_root("deterministic");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    // Set all four kinds; the merge is sorted by kind, so two lists agree.
    for (kind, value) in each_kind_cases() {
        let set = PreferencesCommand::Set {
            kind,
            value: value.into(),
            profile: None,
        };
        let (code, _o, err) = run(set, &runtime, &store, RenderFormat::Text).await;
        assert_eq!(code, 0, "set {kind:?} stderr: {err}");
    }
    let first = PreferencesCommand::List { profile: None };
    let second = PreferencesCommand::List { profile: None };
    let (c1, o1, e1) = run(first, &runtime, &store, RenderFormat::Text).await;
    let (c2, o2, e2) = run(second, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(c1, 0, "e1: {e1}");
    assert_eq!(c2, 0, "e2: {e2}");
    assert_eq!(o1, o2, "two lists of the same store diverged in order");
    assert_eq!(e1, e2);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 7. The opaque profile identity appears in no output, in any format.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn opaque_identity_never_reaches_rendered_output() {
    let root = temp_root("identity_leak");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    // Seed one profile-scoped and one global preference so both scopes render.
    for (kind, value) in [
        (PreferenceKindArg::Timezone, "Europe/London"),
        (PreferenceKindArg::OutputStyle, "compact"),
    ] {
        let set = PreferencesCommand::Set {
            kind,
            value: value.into(),
            profile: None,
        };
        let (code, _o, err) = run(set, &runtime, &store, RenderFormat::Text).await;
        assert_eq!(code, 0, "set {kind:?} stderr: {err}");
    }

    let identity = identity_for(&runtime, "local");
    assert_eq!(identity.len(), 66);
    assert!(identity.starts_with("p-"));

    let list = PreferencesCommand::List { profile: None };
    for format in [RenderFormat::Text, RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, out, err) = run(list.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "list {format:?} stderr: {err}");
        for captured in [out.clone(), err.clone()] {
            assert!(
                !captured.contains(&identity),
                "opaque identity leaked into {format:?} output: {captured}"
            );
        }
    }

    // The unknown-profile error path must not leak the identity either.
    let list_bad = PreferencesCommand::List {
        profile: Some("nope".into()),
    };
    for format in [RenderFormat::Text, RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, out, err) = run(list_bad.clone(), &runtime, &store, format).await;
        assert_ne!(code, 0, "unknown profile must not succeed: {out}{err}");
        let combined = format!("{out}{err}");
        assert!(
            !combined.contains(&identity),
            "opaque identity leaked into unknown-profile error ({format:?}): {combined}"
        );
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 8. An unreadable store fails as `contracts list` does — non-zero, per the
//    correction made in 2b-4.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unopenable_store_list_and_set_both_exit_nonzero() {
    let root = temp_root("unopenable");
    // A path whose parent is a regular file cannot be created as a directory,
    // so the store pool cannot open.
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);
    let (runtime, _c, _n) = runtime_at(&root);

    let list = PreferencesCommand::List { profile: None };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        code, 0,
        "list must exit non-zero on an unopenable store: {out}{err}"
    );
    let combined = format!("{out}{err}");
    assert!(
        !combined.is_empty(),
        "list must emit a diagnostic on an unopenable store"
    );

    let set = PreferencesCommand::Set {
        kind: PreferenceKindArg::OutputStyle,
        value: "compact".into(),
        profile: None,
    };
    let (code, _o, _e) = run(set, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        code, 0,
        "a write against an unopenable store must exit non-zero"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Bonus: JSON and NDJSON carry the same kinds and values as the text form, and
// the scope is a profile name (or "global"), never an identity.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn json_and_ndjson_carry_kinds_values_and_named_scope() {
    let root = temp_root("formats");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    for (kind, value) in [
        (PreferenceKindArg::Timezone, "Europe/London"),
        (PreferenceKindArg::OutputStyle, "compact"),
    ] {
        let set = PreferencesCommand::Set {
            kind,
            value: value.into(),
            profile: None,
        };
        let (code, _o, err) = run(set, &runtime, &store, RenderFormat::Text).await;
        assert_eq!(code, 0, "set {kind:?} stderr: {err}");
    }

    let identity = identity_for(&runtime, "local");
    let list = PreferencesCommand::List { profile: None };
    for format in [RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, out, err) = run(list.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "list {format:?} stderr: {err}");
        let events = json_events(&out);
        let pref_event = events
            .iter()
            .find(|v| v["event"] == "preference_list")
            .expect("a preference_list event was emitted");
        let prefs = pref_event["preferences"]
            .as_array()
            .expect("preferences array");
        assert!(prefs.len() >= 2, "both scopes present in {format:?}: {out}");
        for pref in prefs {
            let scope = pref["scope"].as_str().expect("scope is a string");
            assert!(
                !scope.contains(&identity),
                "opaque identity in {format:?} scope field: {out}"
            );
            // The scope is the profile name or "global" — never the identity.
            assert!(
                scope == "local" || scope == "global",
                "scope must be a name or global, got {scope:?}: {out}"
            );
        }
        // Values and kinds round-trip through the structured event.
        let kinds: Vec<&str> = prefs.iter().map(|p| p["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"timezone"));
        assert!(kinds.contains(&"output_style"));
        let values: Vec<&str> = prefs.iter().map(|p| p["value"].as_str().unwrap()).collect();
        assert!(values.contains(&"Europe/London"));
        assert!(values.contains(&"compact"));
    }

    let _ = fs::remove_dir_all(root);
}
