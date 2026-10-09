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
include!("flow_tests/guided_flow.rs");
include!("flow_tests/recovery.rs");
include!("flow_tests/probe_admission.rs");
include!("flow_tests/backend_profiles.rs");
include!("flow_tests/probe_failures.rs");
include!("flow_tests/review_output.rs");
include!("flow_tests/provider_suggestions.rs");
include!("flow_tests/key_safety.rs");
