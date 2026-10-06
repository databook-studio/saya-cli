//! Tests for the S16 setup adapters: the scripted guided flow, the probes,
//! and the review renderer. Every flow test drives the real engine — plan,
//! commit, recover — against a real temp user directory; prompts come from a
//! `Cursor` script and probes from injected futures, so no network and no
//! terminal is needed.

use super::draft::{ProfileDraft, ProviderDraft};
use super::flow::run_with;
use super::flow_options::FlowOptions;
use super::probe::{DatabaseProbe, FlowProbes, ProbeResult, ProviderProbe};
use super::probe_database::{
    SSO_PROBE_TIMEOUT, classify, database, database_with, needs_sso_consent, probe_timeout_for,
};
use super::probe_provider::{ping_request, provider_with};
use super::review;
use super::{SetupDraft, plan};
use crate::cli::GlobalOptions;
use saya_config::{AiProvider, ConnectionsFile};
use saya_types::{DatabaseProfile, SecretRef, SnowflakeAuth};
use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MARKER_FILE: &str = ".setup-commit.json";
const BACKUP_DIR: &str = ".setup-backup";

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("saya-setup-flow-{}-{label}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Flow options over a temp user directory. `reload: None` means the real
/// engine closure (`load_with_sources` over the same directory with the same
/// empty env) — the commit wiring is exercised, not stubbed.
fn options(dir: &Path) -> FlowOptions {
    FlowOptions {
        options: GlobalOptions::default(),
        user_dir: dir.to_path_buf(),
        cwd: dir.to_path_buf(),
        env: BTreeMap::new(),
        reload: None,
        probes: instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    }
}

/// Probe seams that answer instantly and record each call (`"database:sqlite"`,
/// `"provider:ollama"`), with controllable outcomes.
fn instant_probes(
    calls: &Arc<Mutex<Vec<String>>>,
    database_ok: bool,
    provider_ok: bool,
) -> FlowProbes {
    let db_calls = calls.clone();
    let database: DatabaseProbe = Arc::new(move |profile| {
        db_calls
            .lock()
            .unwrap()
            .push(format!("database:{}", profile.dialect().as_str()));
        let ok = database_ok;
        Box::pin(async move {
            if ok {
                ProbeResult::ok("database reachable")
            } else {
                ProbeResult::failure("database probe failed: connection refused")
            }
        })
    });
    let prov_calls = calls.clone();
    let provider: ProviderProbe = Arc::new(move |draft| {
        prov_calls
            .lock()
            .unwrap()
            .push(format!("provider:{}", draft.provider.as_str()));
        let ok = provider_ok;
        Box::pin(async move {
            if ok {
                ProbeResult::ok("provider answered")
            } else {
                ProbeResult::failure("provider probe failed: refused")
            }
        })
    });
    FlowProbes { database, provider }
}

/// Runs one scripted flow; returns (exit code, captured output).
fn run_flow(dir: &Path, script: &str, probes: FlowProbes) -> (i32, String) {
    let mut opts = options(dir);
    opts.probes = probes;
    let mut input = Cursor::new(script.to_string());
    let mut output = Vec::new();
    let code = run_with(&mut opts, &mut input, &mut output).unwrap();
    (code, String::from_utf8(output).unwrap())
}

/// Skips the provider (6), picks sqlite (1), gives path and name, confirms (y).
const SQLITE_SCRIPT: &str = "6\n1\n/tmp/saya-flow-test.db\nteam\ny\n";

/// The crash state an interrupted commit leaves (mirrors the engine tests).
fn stage_interrupted(dir: &Path, original: &str, rewritten: &str) {
    let marker = serde_json::json!({
        "version": 1,
        "started_unix_ms": 42,
        "entries": [
            { "file": "connections.toml", "backup": "connections.toml", "created": false }
        ]
    });
    fs::write(dir.join(MARKER_FILE), serde_json::to_vec(&marker).unwrap()).unwrap();
    fs::create_dir_all(dir.join(BACKUP_DIR)).unwrap();
    fs::write(dir.join(BACKUP_DIR).join("connections.toml"), original).unwrap();
    fs::write(dir.join("connections.toml"), rewritten).unwrap();
}

#[test]
fn full_sqlite_flow_writes_connections_and_reloads() {
    let dir = temp_dir("full-sqlite");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (code, out) = run_flow(&dir, SQLITE_SCRIPT, instant_probes(&calls, true, true));
    assert_eq!(code, 0, "the flow succeeds: {out}");
    let connections = dir.join("connections.toml");
    assert!(connections.exists(), "connections.toml is written: {out}");
    let parsed = ConnectionsFile::from_toml(&fs::read_to_string(&connections).unwrap()).unwrap();
    assert_eq!(
        parsed.profiles.get("team"),
        Some(&DatabaseProfile::Sqlite {
            path: "/tmp/saya-flow-test.db".into(),
            read_only: true,
        }),
        "the draft profile round-trips through the written file"
    );
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec!["database:sqlite".to_string()],
        "only the database probe runs when no provider was chosen"
    );
    assert!(out.contains("database reachable"), "probe label: {out}");
    assert!(out.contains("configuration valid"), "commit label: {out}");
    assert!(
        out.contains("saya --profile team"),
        "the next step names the new profile: {out}"
    );
    assert!(!dir.join(MARKER_FILE).exists(), "no marker is left");
    assert!(!dir.join(BACKUP_DIR).exists(), "no backups are left");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn cancel_at_confirm_writes_nothing() {
    let dir = temp_dir("cancel-confirm");
    let script = "6\n1\n/tmp/x.db\nteam\nn\n";
    let (code, out) = run_flow(
        &dir,
        script,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "cancelling is not an error: {out}");
    assert!(
        out.contains("No files were changed."),
        "the cancel line: {out}"
    );
    assert!(
        !dir.join("connections.toml").exists(),
        "nothing was written"
    );
    assert!(!dir.join(MARKER_FILE).exists(), "no marker is left");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn eof_mid_flow_cancels_with_no_files() {
    let dir = temp_dir("eof-mid");
    // Provider skipped, then EOF at the database menu.
    let (code, out) = run_flow(
        &dir,
        "6\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "EOF cancels, never an error: {out}");
    assert!(out.contains("No files were changed."), "{out}");
    assert!(!dir.join("connections.toml").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn eof_at_the_first_prompt_cancels_with_no_files() {
    let dir = temp_dir("eof-first");
    let (code, out) = run_flow(
        &dir,
        "",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("No files were changed."), "{out}");
    assert!(!dir.join("connections.toml").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn existing_connections_shows_only_the_appended_block() {
    let dir = temp_dir("append");
    let original = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.db\"\nread_only = true\n";
    fs::write(dir.join("connections.toml"), original).unwrap();
    let script = "6\n1\n/tmp/b.db\nb\ny\n";
    let (code, out) = run_flow(
        &dir,
        script,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("[profiles.b]"),
        "the appended block is shown: {out}"
    );
    assert!(
        !out.contains("[profiles.a]"),
        "the review shows only the appended block, never the whole file: {out}"
    );
    let parsed =
        ConnectionsFile::from_toml(&fs::read_to_string(dir.join("connections.toml")).unwrap())
            .unwrap();
    assert!(
        parsed.profiles.contains_key("a") && parsed.profiles.contains_key("b"),
        "the commit appends without dropping the existing profile"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn interrupted_marker_restore_path_restores_originals() {
    let dir = temp_dir("rec-restore");
    let original = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.db\"\nread_only = true\n";
    let rewritten = format!(
        "{original}\n[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n"
    );
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted(&dir, original, &rewritten);

    // r = restore, then skip the provider (6) and the database (8).
    let (code, out) = run_flow(
        &dir,
        "r\n6\n8\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read_to_string(dir.join("connections.toml")).unwrap(),
        original,
        "restore put the original bytes back"
    );
    assert!(!dir.join(MARKER_FILE).exists(), "the marker is cleared");
    assert!(!dir.join(BACKUP_DIR).exists(), "the backups are cleared");
    assert!(
        out.contains("Restored the previous files"),
        "the flow says the restore happened: {out}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn interrupted_marker_finish_keeps_new_bytes() {
    let dir = temp_dir("rec-finish");
    let original = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.db\"\nread_only = true\n";
    let rewritten = format!(
        "{original}\n[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n"
    );
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted(&dir, original, &rewritten);

    let (code, out) = run_flow(
        &dir,
        "f\n6\n8\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read_to_string(dir.join("connections.toml")).unwrap(),
        rewritten,
        "finish keeps the current files"
    );
    assert!(!dir.join(MARKER_FILE).exists(), "the marker is cleared");
    assert!(out.contains("Kept the current files"), "{out}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn interrupted_marker_quit_changes_nothing() {
    let dir = temp_dir("rec-quit");
    let original = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.db\"\nread_only = true\n";
    let rewritten = format!(
        "{original}\n[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n"
    );
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted(&dir, original, &rewritten);

    let (code, out) = run_flow(
        &dir,
        "q\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read_to_string(dir.join("connections.toml")).unwrap(),
        rewritten,
        "quit neither restores nor writes"
    );
    assert!(dir.join(MARKER_FILE).exists(), "quit leaves the marker");
    let _ = fs::remove_dir_all(dir);
}

/// A restore that does not complete never prints the success text: the flow
/// prints each failure and the marker-kept guidance, then fails (exit 2 via
/// the propagated error), leaving the pending state for a fixed retry.
#[test]
fn restore_failure_prints_failures_and_never_the_success_text() {
    let dir = temp_dir("rec-restore-fail");
    let original = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.db\"\nread_only = true\n";
    fs::write(dir.join("connections.toml"), original).unwrap();
    // Two-entry marker (mirrors the engine tests): connections.toml with a
    // present backup, and the created config.toml — replaced by a directory,
    // so removing it fails.
    let marker = serde_json::json!({
        "version": 1,
        "started_unix_ms": 42,
        "entries": [
            { "file": "connections.toml", "backup": "connections.toml", "created": false },
            { "file": "config.toml", "backup": null, "created": true }
        ]
    });
    fs::write(dir.join(MARKER_FILE), serde_json::to_vec(&marker).unwrap()).unwrap();
    fs::create_dir_all(dir.join(BACKUP_DIR)).unwrap();
    fs::write(dir.join(BACKUP_DIR).join("connections.toml"), original).unwrap();
    fs::create_dir_all(dir.join("config.toml")).unwrap();

    let mut opts = options(&dir);
    let mut input = Cursor::new("r\n");
    let mut output = Vec::new();
    let result = run_with(&mut opts, &mut input, &mut output);
    let error = result.expect_err("an incomplete restore is an error");
    assert!(
        error.to_string().contains("restore did not complete"),
        "the error says the restore did not complete: {error}"
    );
    let out = String::from_utf8(output).unwrap();
    assert!(
        !out.contains("Restored the previous files"),
        "the success text is never printed on failure: {out}"
    );
    assert!(
        out.contains("The recovery marker was kept; fix the file and run `saya setup` again."),
        "the marker-kept guidance is printed: {out}"
    );
    assert!(
        out.contains("config.toml"),
        "each failure names its file: {out}"
    );
    assert!(dir.join(MARKER_FILE).exists(), "the marker is kept");
    assert!(dir.join(BACKUP_DIR).exists(), "the backups are kept");
    assert!(
        !out.contains("No files were changed."),
        "the flow stops; no false cancel line: {out}"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A recovery marker that cannot be read stops the flow at once, naming the
/// marker path and the manual way out: `commit` would refuse the pending
/// state only at the very end, after the whole flow has been answered. The
/// marker is never deleted automatically.
#[test]
fn unreadable_marker_stops_the_flow_naming_the_way_out() {
    let dir = temp_dir("rec-unreadable");
    let marker_path = dir.join(MARKER_FILE);
    fs::write(&marker_path, b"{ interrupted, but corrupt").unwrap();

    let mut opts = options(&dir);
    // A full script: if the flow continued, it would run to the commit that
    // refuses at the very end.
    let mut input = Cursor::new(SQLITE_SCRIPT);
    let mut output = Vec::new();
    let result = run_with(&mut opts, &mut input, &mut output);

    let error = result.expect_err("an unreadable marker stops the flow");
    let message = error.to_string();
    let marker = marker_path.to_str().unwrap();
    assert!(
        message.contains(marker),
        "the message names the marker path: {message}"
    );
    assert!(
        message.contains("inspect or remove") && message.contains("then run `saya setup` again"),
        "the message names the manual way out: {message}"
    );
    let out = String::from_utf8(output).unwrap();
    assert!(
        !out.contains("database") && !out.contains("Nothing to configure"),
        "the flow stopped before any prompt: {out}"
    );
    assert!(
        marker_path.exists(),
        "the marker is never deleted automatically"
    );
    assert!(
        !dir.join("connections.toml").exists() && !dir.join("config.toml").exists(),
        "nothing was written"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn three_invalid_answers_cancel() {
    let dir = temp_dir("three-bad");
    let (code, out) = run_flow(
        &dir,
        "bogus\nbogus\nbogus\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "three strikes cancel, not error: {out}");
    assert!(out.contains("No files were changed."), "{out}");
    assert!(!dir.join("connections.toml").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn failed_database_probe_does_not_block_writing() {
    let dir = temp_dir("probe-fail");
    let (code, out) = run_flow(
        &dir,
        SQLITE_SCRIPT,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), false, true),
    );
    assert_eq!(code, 0, "a failed probe does not block writing: {out}");
    assert!(
        out.contains("database probe failed: connection refused"),
        "the failure is shown verbatim: {out}"
    );
    assert!(out.contains("configuration valid"), "{out}");
    assert!(
        dir.join("connections.toml").exists(),
        "the file was written"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn provider_probe_needs_consent_and_skips_without_it() {
    let dir = temp_dir("consent-no");
    // Ollama (1), model blank (default), base URL blank (default); sqlite,
    // path, name; decline the provider probe (n); confirm the write (y).
    let script = "1\n\n\n1\n/tmp/x.db\nteam\nn\ny\n";
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (code, out) = run_flow(&dir, script, instant_probes(&calls, true, true));
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec!["database:sqlite".to_string()],
        "the provider probe never runs without consent"
    );
    assert!(out.contains("Provider probe skipped"), "{out}");
    assert!(dir.join("connections.toml").exists());
    assert!(
        dir.join("config.toml").exists(),
        "the ollama section is written"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn provider_probe_runs_with_consent() {
    let dir = temp_dir("consent-yes");
    let script = "1\n\n\n1\n/tmp/x.db\nteam\ny\ny\n";
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (code, out) = run_flow(&dir, script, instant_probes(&calls, true, true));
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec!["database:sqlite".to_string(), "provider:ollama".to_string()],
        "the consented probe runs after the database probe"
    );
    assert!(out.contains("provider answered"), "{out}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn skipping_everything_changes_nothing() {
    let dir = temp_dir("skip-all");
    let (code, out) = run_flow(
        &dir,
        "6\n8\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Nothing to configure"), "{out}");
    assert!(!dir.join("connections.toml").exists());
    assert!(!dir.join("config.toml").exists());
}

// ------------------------------------------------------------ warehouses ----

/// A ClickHouse flow end to end: TLS yes, defaults taken for port, database
/// and user skipped, the password only as an environment-variable name.
#[test]
fn clickhouse_flow_writes_the_expected_profile() {
    let dir = temp_dir("flow-clickhouse");
    let script = "6\n6\nlocalhost\ny\n\n\n\nSAYA_CH_PASSWORD\nch\ny\n";
    let (code, out) = run_flow(
        &dir,
        script,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    let parsed =
        ConnectionsFile::from_toml(&fs::read_to_string(dir.join("connections.toml")).unwrap())
            .unwrap_or_else(|error| panic!("the written file parses: {error}"));
    assert_eq!(
        parsed.profiles.get("ch"),
        Some(&DatabaseProfile::ClickHouse {
            host: "localhost".into(),
            port: None,
            database: None,
            user: None,
            password: Some(SecretRef::Env {
                env: "SAYA_CH_PASSWORD".into()
            }),
            secure: Some(true),
        }),
        "the scripted flow wrote the expected profile"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A BigQuery flow end to end: required project and key path, optionals
/// skipped. The key is stored as a file reference only.
#[test]
fn bigquery_flow_writes_the_expected_profile() {
    let dir = temp_dir("flow-bigquery");
    let script = "6\n7\nmy-project\n\n\n\n/keys/sa.json\nbq\ny\n";
    let (code, out) = run_flow(
        &dir,
        script,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    let parsed =
        ConnectionsFile::from_toml(&fs::read_to_string(dir.join("connections.toml")).unwrap())
            .unwrap_or_else(|error| panic!("the written file parses: {error}"));
    assert_eq!(
        parsed.profiles.get("bq"),
        Some(&DatabaseProfile::BigQuery {
            project: "my-project".into(),
            dataset: None,
            location: None,
            max_bytes_billed: None,
            service_account_key: SecretRef::File {
                file: "/keys/sa.json".into()
            },
        }),
        "the scripted flow wrote the expected profile"
    );
    let _ = fs::remove_dir_all(dir);
}

/// Declining the browser-SSO consent skips the probe entirely (no probe call,
/// no probing line) and labels it "not probed"; the flow still completes.
#[test]
fn snowflake_sso_probe_declined_skips_and_labels_not_probed() {
    let dir = temp_dir("sso-declined");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let script = "6\n5\nacct\njane\n2\n\n\n\n\n\nn\ny\n";
    let (code, out) = run_flow(&dir, script, instant_probes(&calls, true, true));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("not probed"), "the skip is labelled: {out}");
    let recorded = calls.lock().unwrap().clone();
    assert!(
        !recorded.iter().any(|call| call == "database:snowflake"),
        "declining skips the probe: {recorded:?}"
    );
    assert!(
        !out.contains("Probing the database"),
        "no probing line when skipped: {out}"
    );
    let parsed =
        ConnectionsFile::from_toml(&fs::read_to_string(dir.join("connections.toml")).unwrap())
            .unwrap();
    assert_eq!(
        parsed.profiles.get("snowflake"),
        Some(&DatabaseProfile::Snowflake {
            account: "acct".into(),
            user: "jane".into(),
            auth_type: SnowflakeAuth::Externalbrowser,
            private_key: None,
            password: None,
            passphrase: None,
            warehouse: None,
            database: None,
            schema: None,
            role: None,
        })
    );
    let _ = fs::remove_dir_all(dir);
}

/// Consenting runs the probe with the 120-second SSO window named up front.
#[test]
fn snowflake_sso_probe_consented_runs_with_the_120s_window() {
    let dir = temp_dir("sso-consented");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let script = "6\n5\nacct\njane\n2\n\n\n\n\n\ny\ny\n";
    let (code, out) = run_flow(&dir, script, instant_probes(&calls, true, true));
    assert_eq!(code, 0, "{out}");
    let recorded = calls.lock().unwrap().clone();
    assert!(
        recorded.iter().any(|call| call == "database:snowflake"),
        "consent runs the probe: {recorded:?}"
    );
    assert!(
        out.contains("Probing the database (up to 120 seconds)"),
        "the SSO window is announced: {out}"
    );
    assert!(out.contains("database reachable"), "{out}");
    let _ = fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------- probes ----

#[tokio::test]
async fn setup_timeout_names_failed_probe() {
    let result = database_with(std::time::Duration::from_millis(30), || {
        std::future::pending()
    })
    .await;
    assert!(!result.ok, "a never-resolving connection fails");
    assert!(
        result.message.contains("database"),
        "the message names the database probe: {}",
        result.message
    );
    assert!(
        result.message.contains("timed out after 0.0s"),
        "the message names the timeout: {}",
        result.message
    );
}

#[tokio::test]
async fn provider_probe_timeout_is_named() {
    let result = provider_with(std::time::Duration::from_millis(30), || {
        std::future::pending::<Result<(), String>>()
    })
    .await;
    assert!(!result.ok);
    assert!(
        result.message.contains("provider probe timed out"),
        "{}",
        result.message
    );
}

#[tokio::test]
async fn probe_failure_keeps_the_underlying_message() {
    let result = database_with(std::time::Duration::from_millis(30), || async {
        Err("auth failed: bad password".to_string())
    })
    .await;
    assert!(!result.ok);
    assert!(
        result.message.contains("auth failed: bad password"),
        "the error message is always included: {}",
        result.message
    );
}

/// Only Snowflake browser-SSO needs the consent gate and the 120-second
/// window; keypair, userpass, and every other engine keep the 15-second one.
#[test]
fn sso_window_and_consent_apply_only_to_browser_auth() {
    let browser = DatabaseProfile::Snowflake {
        account: "a".into(),
        user: "u".into(),
        auth_type: SnowflakeAuth::Externalbrowser,
        private_key: None,
        password: None,
        passphrase: None,
        warehouse: None,
        database: None,
        schema: None,
        role: None,
    };
    let keypair = DatabaseProfile::Snowflake {
        account: "a".into(),
        user: "u".into(),
        auth_type: SnowflakeAuth::Keypair,
        private_key: None,
        password: None,
        passphrase: None,
        warehouse: None,
        database: None,
        schema: None,
        role: None,
    };
    let clickhouse = DatabaseProfile::ClickHouse {
        host: "h".into(),
        port: None,
        database: None,
        user: None,
        password: None,
        secure: None,
    };
    assert!(needs_sso_consent(&browser));
    assert!(!needs_sso_consent(&keypair));
    assert!(!needs_sso_consent(&clickhouse));
    assert_eq!(probe_timeout_for(&browser), SSO_PROBE_TIMEOUT);
    assert_eq!(probe_timeout_for(&keypair), super::probe::PROBE_TIMEOUT);
    assert_eq!(probe_timeout_for(&clickhouse), super::probe::PROBE_TIMEOUT);
    assert_eq!(SSO_PROBE_TIMEOUT, std::time::Duration::from_secs(120));
}

/// The real probe path with a secret that cannot be resolved: the factory
/// refuses at build time, offline, and the failure names the cause.
#[tokio::test]
async fn database_probe_reports_an_unresolvable_secret() {
    let profile = DatabaseProfile::Snowflake {
        account: "org-account".into(),
        user: "jane".into(),
        auth_type: SnowflakeAuth::Keypair,
        private_key: Some(SecretRef::Env {
            env: "SAYA_SETUP_TEST_MISSING_KEY".into(),
        }),
        password: None,
        passphrase: None,
        warehouse: None,
        database: None,
        schema: None,
        role: None,
    };
    let resolver = saya_config::MapSecretResolver::new(BTreeMap::new());
    let result = database(&profile, &resolver).await;
    assert!(!result.ok, "an unresolvable secret fails the probe");
    assert!(
        result.message.contains("could not be resolved"),
        "the cause is named: {}",
        result.message
    );
}

#[test]
fn provider_probe_sends_no_database_data() {
    let request = ping_request("test-model");
    assert_eq!(request.messages.len(), 1, "exactly one message");
    assert_eq!(
        request.messages[0].role, "user",
        "a user message, not system"
    );
    assert_eq!(request.messages[0].content, "ping", "the word ping, only");
    assert!(request.tools.is_empty(), "no tool definitions attached");
    assert!(
        !request
            .messages
            .iter()
            .any(|message| message.role == "system"),
        "no system message, so no schema can ride along"
    );
    assert_eq!(request.model, "test-model");
}

#[test]
fn connection_errors_classify_best_effort() {
    let classified = classify("password authentication failed for user");
    assert!(classified.contains("authentication failed"), "{classified}");
    let classified = classify("connection timed out");
    assert!(classified.contains("timed out"), "{classified}");
    let classified = classify("SSL error: certificate verify failed");
    assert!(classified.contains("tls"), "{classified}");
    let classified = classify("database \"nope\" does not exist");
    assert!(classified.contains("not found"), "{classified}");
    let classified = classify("something odd happened");
    assert!(classified.contains("could not connect"), "{classified}");
    assert!(
        classified.contains("something odd happened"),
        "the original error is always included: {classified}"
    );
}

// ---------------------------------------------------------------- review ----

#[test]
fn review_prints_notes_and_created_files() {
    let dir = temp_dir("review");
    fs::write(dir.join("config.toml"), "[ai]\nprovider = \"ollama\"\n").unwrap();
    let draft = SetupDraft {
        provider: Some(ProviderDraft {
            provider: AiProvider::Openai,
            model: "gpt-4o-mini".into(),
            base_url: None,
            api_key_env: Some("OPENAI_API_KEY".into()),
        }),
        profile: None,
    };
    let planned = plan(&dir, &draft).unwrap();
    assert!(planned.writes.is_empty(), "config.toml exists: note only");
    let text = review::render(&dir, &planned);
    assert!(text.contains("note:"), "the note is printed: {text}");
    assert!(
        text.contains("api_key = { env = \"OPENAI_API_KEY\" }"),
        "the note carries the exact snippet: {text}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn review_prints_the_full_text_for_a_new_file() {
    let dir = temp_dir("review-create");
    let draft = SetupDraft {
        provider: None,
        profile: Some(ProfileDraft {
            name: "team".into(),
            profile: DatabaseProfile::Sqlite {
                path: "/tmp/t.db".into(),
                read_only: true,
            },
        }),
    };
    let planned = plan(&dir, &draft).unwrap();
    let text = review::render(&dir, &planned);
    assert!(text.contains("create"), "the action is named: {text}");
    assert!(text.contains("[profiles.team]"), "the block: {text}");
    assert!(
        text.contains("read_only = true"),
        "the exact new text: {text}"
    );
    let _ = fs::remove_dir_all(dir);
}

// ------------------------------------------- provider key reference rules ----

/// Completes a provider-only script: skip the database (8), decline the
/// provider probe, confirm the write.
fn provider_tail(script_head: &str) -> String {
    format!("{script_head}8\nn\ny\n")
}

/// The config.toml the flow wrote, or a panic that shows the flow output.
fn written_config(dir: &Path, out: &str) -> String {
    fs::read_to_string(dir.join("config.toml"))
        .unwrap_or_else(|error| panic!("config.toml was written: {error}; output:\n{out}"))
}

fn key_flow(dir: &Path, script: &str) -> (i32, String) {
    run_flow(
        dir,
        script,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    )
}

/// A cloud provider suggests exactly its own conventional key NAME; a blank
/// answer accepts it; the written reference is a name, never a value.
#[test]
fn openai_suggests_its_own_key_name_and_a_blank_answer_accepts_it() {
    let dir = temp_dir("key-openai");
    let (code, out) = key_flow(&dir, &provider_tail("2\n\n\n"));
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("API key environment variable name [OPENAI_API_KEY] (never the key itself): "),
        "{out}"
    );
    assert!(
        out.contains("OPENAI_API_KEY is not set.")
            || out.contains("OPENAI_API_KEY is currently set."),
        "the presence line names the variable only: {out}"
    );
    let config = written_config(&dir, &out);
    assert!(config.contains("provider = \"openai\""), "{config}");
    assert!(
        config.contains("api_key = { env = \"OPENAI_API_KEY\" }"),
        "{config}"
    );
    assert!(
        !config.contains("base_url"),
        "openai carries no base URL: {config}"
    );
    assert!(
        !config.contains("api_key = \""),
        "never an inline value: {config}"
    );
    assert!(!out.contains("sk-"), "no secret-looking value shown: {out}");
    let _ = fs::remove_dir_all(dir);
}

/// Anthropic and Gemini each suggest only their own name, never another
/// provider's.
#[test]
fn anthropic_and_gemini_suggest_their_own_names() {
    for (choice, provider, name, others) in [
        (
            "4",
            "anthropic",
            "ANTHROPIC_API_KEY",
            ["OPENAI_API_KEY", "GEMINI_API_KEY"],
        ),
        (
            "5",
            "gemini",
            "GEMINI_API_KEY",
            ["OPENAI_API_KEY", "ANTHROPIC_API_KEY"],
        ),
    ] {
        let dir = temp_dir(&format!("key-{provider}"));
        let (code, out) = key_flow(&dir, &provider_tail(&format!("{choice}\n\n\n")));
        assert_eq!(code, 0, "{provider}: {out}");
        assert!(
            out.contains(&format!(
                "API key environment variable name [{name}] (never the key itself): "
            )),
            "{provider}: {out}"
        );
        for other in others {
            assert!(
                !out.contains(other),
                "{provider} never shows {other}: {out}"
            );
        }
        let config = written_config(&dir, &out);
        assert!(
            config.contains(&format!("provider = \"{provider}\"")),
            "{config}"
        );
        assert!(
            config.contains(&format!("api_key = {{ env = \"{name}\" }}")),
            "{config}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

/// A compatible endpoint gets no suggested name and a blank answer skips the
/// key entirely: no reference, no presence line.
#[test]
fn a_compatible_endpoint_gets_no_suggested_key_and_a_blank_answer_skips_it() {
    let dir = temp_dir("key-compat-blank");
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\n\n"),
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("API key environment variable name (blank to skip; never the key itself): "),
        "{out}"
    );
    assert!(
        !out.contains("API key environment variable name ["),
        "no bracketed suggestion for a compatible endpoint: {out}"
    );
    assert!(
        !out.contains(" is not set.") && !out.contains(" is currently set."),
        "a skipped key prints no presence line: {out}"
    );
    assert!(!out.contains("OPENAI_API_KEY"), "{out}");
    let config = written_config(&dir, &out);
    assert!(
        config.contains("base_url = \"https://compat.example.com/v1\""),
        "{config}"
    );
    assert!(
        !config.contains("api_key"),
        "a blank answer skips the key: {config}"
    );
    assert!(!config.contains("OPENAI_API_KEY"), "{config}");
    let _ = fs::remove_dir_all(dir);
}

/// A compatible endpoint writes exactly the env NAME the user typed.
#[test]
fn a_compatible_endpoint_uses_only_the_name_the_user_types() {
    let dir = temp_dir("key-compat-named");
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\nSAYA_TEST_UNSET_KEY_7F3A91\n"),
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("SAYA_TEST_UNSET_KEY_7F3A91 is not set."),
        "the presence line reports the name only: {out}"
    );
    let config = written_config(&dir, &out);
    assert!(
        config.contains("api_key = { env = \"SAYA_TEST_UNSET_KEY_7F3A91\" }"),
        "{config}"
    );
    assert!(
        config.contains("base_url = \"https://compat.example.com/v1\""),
        "{config}"
    );
    assert!(!config.contains("OPENAI_API_KEY"), "{config}");
    let _ = fs::remove_dir_all(dir);
}

/// Between the key prompt and the next prompt the flow prints exactly the
/// presence sentence — no echoed value, nothing else.
#[test]
fn the_presence_line_never_prints_a_value() {
    let dir = temp_dir("key-presence");
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\nSAYA_TEST_UNSET_KEY_7F3A91\n"),
    );
    assert_eq!(code, 0, "{out}");
    let prompt = "API key environment variable name (blank to skip; never the key itself): ";
    let start = out.find(prompt).expect("the key prompt is shown") + prompt.len();
    let end = out
        .find("Choose a database:")
        .expect("the database menu follows the provider");
    assert_eq!(
        &out[start..end],
        "SAYA_TEST_UNSET_KEY_7F3A91 is not set.\n",
        "only the name and the presence sentence sit between the prompts"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A malformed name is refused once and re-asked; three invalid answers burn
/// the bounded retry and cancel with nothing written.
#[test]
fn a_malformed_key_name_is_refused_and_reasked() {
    let dir = temp_dir("key-malformed");
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\nmy-key\nSAYA_TEST_UNSET_KEY_7F3A91\n"),
    );
    assert_eq!(code, 0, "{out}");
    let reason = "api key environment variable name \"my-key\" must match [A-Z_][A-Z0-9_]{0,63}";
    assert_eq!(
        out.matches(reason).count(),
        1,
        "the refusal is said once, then the prompt is asked again: {out}"
    );
    assert!(
        out.contains("SAYA_TEST_UNSET_KEY_7F3A91 is not set."),
        "the flow continues after the refusal: {out}"
    );
    let config = written_config(&dir, &out);
    assert!(
        config.contains("api_key = { env = \"SAYA_TEST_UNSET_KEY_7F3A91\" }"),
        "{config}"
    );
    let _ = fs::remove_dir_all(dir);

    let dir = temp_dir("key-malformed-cancel");
    let (code, out) = key_flow(
        &dir,
        "3\n\nhttps://compat.example.com/v1\nmy-key\nmy-key\nmy-key\n",
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("No files were changed."), "{out}");
    assert!(
        !dir.join("config.toml").exists(),
        "a cancelled flow writes nothing"
    );
    let _ = fs::remove_dir_all(dir);
}

/// Ollama is local: no key question at all, and no key in the written section.
#[test]
fn ollama_never_asks_for_a_key() {
    let dir = temp_dir("key-ollama");
    let (code, out) = key_flow(&dir, &provider_tail("1\n\n\n"));
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("API key environment variable name"), "{out}");
    let config = written_config(&dir, &out);
    assert!(config.contains("provider = \"ollama\""), "{config}");
    assert!(
        config.contains("base_url = \"http://localhost:11434\""),
        "{config}"
    );
    assert!(!config.contains("api_key"), "{config}");
    let _ = fs::remove_dir_all(dir);
}

/// Whatever the wizard collects, an existing config.toml keeps its bytes: the
/// new endpoint's section goes into the note only, without the old key.
#[test]
fn an_existing_config_is_never_rewritten_whatever_provider_is_chosen() {
    let existing =
        b"[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o-mini\"\napi_key = { env = \"OPENAI_API_KEY\" }\n"
            .to_vec();

    let dir = temp_dir("config-kept-blank");
    fs::write(dir.join("config.toml"), &existing).unwrap();
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\n\n"),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read(dir.join("config.toml")).unwrap(),
        existing,
        "the existing config bytes are never rewritten"
    );
    assert!(
        out.contains("note:"),
        "the section is offered as a note: {out}"
    );
    assert!(
        out.contains("base_url = \"https://compat.example.com/v1\""),
        "{out}"
    );
    assert!(
        !out.contains("api_key"),
        "the old key reference is not carried into the new endpoint's snippet: {out}"
    );
    assert!(!out.contains("OPENAI_API_KEY"), "{out}");
    let _ = fs::remove_dir_all(dir);

    let dir = temp_dir("config-kept-named");
    fs::write(dir.join("config.toml"), &existing).unwrap();
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\nSAYA_TEST_UNSET_KEY_7F3A91\n"),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(fs::read(dir.join("config.toml")).unwrap(), existing);
    assert!(out.contains("note:"), "{out}");
    assert!(
        out.contains("api_key = { env = \"SAYA_TEST_UNSET_KEY_7F3A91\" }"),
        "the note carries the name the user typed: {out}"
    );
    assert!(!out.contains("OPENAI_API_KEY"), "{out}");
    let _ = fs::remove_dir_all(dir);
}

/// The review shows the exact `api_key` reference before the write is
/// confirmed, so the user sees what will land in config.toml.
#[test]
fn the_review_shows_the_exact_reference_it_will_write() {
    let dir = temp_dir("key-review");
    let (code, out) = key_flow(&dir, &provider_tail("2\n\n\n"));
    assert_eq!(code, 0, "{out}");
    let confirmed = out
        .find("Write these files?")
        .expect("the confirm is asked");
    let review = &out[..confirmed];
    assert!(
        review.contains("api_key = { env = \"OPENAI_API_KEY\" }"),
        "the user sees the exact reference before writing: {review}"
    );
    assert!(review.contains("provider = \"openai\""), "{review}");
    let _ = fs::remove_dir_all(dir);
}
