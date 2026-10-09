//! Unit tests for `config doctor --task`: per-task readiness computed from
//! the resolved configuration only — no network, no database connection, and
//! no secret value ever serialized. The runtime fixtures hand-build a
//! `RuntimeConfig` the same way `doctor.rs`'s `test_report` does.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{CheckStatus, DoctorTask, TaskCheck, TaskReadiness, readiness};
use crate::config::doctor;
use crate::config::runtime::RuntimeConfig;
use crate::render::{RenderFormat, TerminalEvent, render_event};

fn runtime_from(
    config_toml: &str,
    connections_toml: &str,
    secrets: BTreeMap<String, String>,
) -> RuntimeConfig {
    use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
    let config = ConfigFile::from_toml(config_toml).unwrap();
    let connections = ConnectionsFile::from_toml(connections_toml).unwrap();
    let resolved = resolve(ResolutionInput::new(connections.clone()).with_user(config)).unwrap();
    RuntimeConfig {
        resolved,
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: std::path::PathBuf::from("/tmp/saya-doctor-task-test"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: secrets,
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-doctor-task-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn check_named<'a>(readiness: &'a TaskReadiness, name: &str) -> &'a TaskCheck {
    readiness
        .checks
        .iter()
        .find(|check| check.name == name)
        .unwrap_or_else(|| panic!("no `{name}` check in {readiness:?}"))
}

// --- query: readiness equals the legacy `can_run_query` decision ---

fn query_runtime(profile_toml: Option<&str>) -> RuntimeConfig {
    match profile_toml {
        Some(toml) => runtime_from(
            "default_profile = \"analytics\"\n",
            &format!("[profiles.analytics]\n{toml}"),
            BTreeMap::new(),
        ),
        None => runtime_from("", "", BTreeMap::new()),
    }
}

include!("doctor_task_tests/query_readiness.rs");

// --- ai: provider, credentials, endpoint ---

include!("doctor_task_tests/ai_readiness.rs");

// --- coding: ai checks plus workspace; the database is not consulted ---

fn inside_a_worktree(start: &Path) -> bool {
    let mut current = Some(start.to_path_buf());
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return true;
        }
        current = dir.parent().map(Path::to_path_buf);
    }
    false
}

include!("doctor_task_tests/coding_and_serialization.rs");

// --- the TerminalEvent text shape, through the public render seam ---

fn event_from(readiness: &TaskReadiness) -> TerminalEvent {
    TerminalEvent::DoctorTask {
        task: readiness.task.to_string(),
        ready: readiness.ready,
        checks: readiness.checks.clone(),
    }
}

include!("doctor_task_tests/text_shapes.rs");

// --- edit hints: a failing or warning check says what to edit ---

fn user_config_file() -> String {
    crate::config::sources::user_config_dir()
        .join("config.toml")
        .display()
        .to_string()
}

include!("doctor_task_tests/hints.rs");
