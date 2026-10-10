//! The acquire-time crash recovery: while the session lock makes this
//! process the only owner of `<id>`, one bounded recovery pass converts
//! leftover `Prepared` records to `Uncertain` and removes orphan
//! before-images — and a session with no checkpoints directory acquires
//! unchanged, creating none.

use std::fs;
use std::path::{Path, PathBuf};

use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_store::{FsSessionStore, NewCheckpoint};
use saya_types::{CheckpointPolicy, CheckpointState, EditSource, RestoreEligibility};

use super::SessionRuntime;
use crate::commands::recovery::{StatusNow, list_changes};

fn temp_root(tag: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-recover-acquire-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    root
}

fn test_runtime() -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            coding: Default::default(),
            instructions: Default::default(),
            profile_name: None,
            profile: None,
            investigation_budgets: Default::default(),
            ai: ResolvedAi {
                provider: AiProvider::Ollama,
                model: "test-model".into(),
                base_url: None,
                api_key: None,
                allow_data_sharing: true,
                temperature: 0.0,
                timeout_seconds: 60,
                idle_timeout_seconds: 90,
                max_output_tokens: 4096,
                max_output_tokens_is_default: true,
                context_byte_budget: 256 * 1024,
                context_window_tokens: None,
                show_thinking: false,
                compaction: saya_config::CompactionMode::Auto,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: Default::default(),
                turns: Some(4),
                tool_calls: None,
                fetch: ResolvedFetchJobs::default(),
                interpreter: ResolvedInterpreterJobs::default(),
                runner: ResolvedRunnerJobs::default(),
            },
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            ui_theme: ThemeChoice::Auto,
            memory: ResolvedMemory {
                mode: MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            provenance: Default::default(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-recover-acquire"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

fn acquire(sessions_root: &Path, id: &str) -> SessionRuntime {
    SessionRuntime::acquire(
        &test_runtime(),
        None,
        false,
        None,
        id,
        ApprovalPolicy::Ask,
        sessions_root,
    )
    .expect("the session acquires")
}

fn checkpoints_dir(sessions_root: &Path, id: &str) -> PathBuf {
    sessions_root.join(id).join("checkpoints")
}

/// A crash between prepare and commit: one `Prepared` checkpoint and one
/// orphan `<id>.before` whose record never landed.
fn seed_crash(store: &FsSessionStore, sessions_root: &Path, session_id: &str) {
    let policy = CheckpointPolicy { enabled: true };
    let prepared = store
        .prepare_checkpoint(
            session_id,
            &policy,
            NewCheckpoint {
                path: "notes.txt",
                source: EditSource::WorkspaceEdit,
                before: Some(b"before bytes"),
            },
            1_000,
        )
        .expect("the first checkpoint prepares");
    assert_eq!(prepared.state, CheckpointState::Prepared);
    let orphan = store
        .prepare_checkpoint(
            session_id,
            &policy,
            NewCheckpoint {
                path: "lost.txt",
                source: EditSource::WorkspaceWrite,
                before: Some(b"orphan bytes"),
            },
            2_000,
        )
        .expect("the second checkpoint prepares");
    fs::remove_file(
        checkpoints_dir(sessions_root, session_id).join(format!("{}.json", orphan.id.as_str())),
    )
    .expect("the record is removed, leaving its image an orphan");
}

#[test]
fn an_acquire_converts_the_prepared_record_and_clears_the_orphan_image() {
    let root = temp_root("crash");
    let sessions_root = root.join("sessions");
    let store = FsSessionStore::new(&sessions_root);
    seed_crash(&store, &sessions_root, "recover-session");

    let _session = acquire(&sessions_root, "recover-session");

    let records = store
        .list_checkpoints("recover-session")
        .expect("the records list");
    assert_eq!(records.len(), 1, "exactly one record remains");
    assert_eq!(
        records[0].state,
        CheckpointState::Uncertain,
        "the crash's Prepared record is never restorable"
    );
    let dir = checkpoints_dir(&sessions_root, "recover-session");
    let entries: Vec<String> = fs::read_dir(&dir)
        .expect("the checkpoints dir lists")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    assert_eq!(
        entries
            .iter()
            .filter(|name| name.ends_with(".before"))
            .count(),
        1,
        "the orphan image is gone; the prepared record's image stays: {entries:?}"
    );

    let view = list_changes(&store, "recover-session", None).expect("the /recovery list view");
    assert_eq!(view.items.len(), 1);
    assert_eq!(
        view.items[0].restore,
        StatusNow::Known(RestoreEligibility::EffectUncertain),
        "the /recovery list reports the uncertain outcome"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_acquire_over_a_session_with_no_checkpoints_creates_no_checkpoints_dir() {
    let root = temp_root("no-checkpoints");
    let sessions_root = root.join("sessions");

    let _session = acquire(&sessions_root, "fresh-session");

    assert!(
        !checkpoints_dir(&sessions_root, "fresh-session").exists(),
        "a cheap no-op recovery pass never creates the checkpoints directory"
    );
    let _ = fs::remove_dir_all(&root);
}
