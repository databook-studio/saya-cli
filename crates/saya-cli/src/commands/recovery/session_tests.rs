//! Tests for the `/recovery` surface operation: the one `run_recovery`
//! every surface runs (status, on, off, list, diff, restore), the
//! id-prefix resolution, and the end-to-end path — a real composed
//! universe whose executor records through the session's own handle,
//! flipped by `run_recovery(On)` and undone by `run_recovery(Restore)`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::CancellationToken;
use saya_types::{CheckpointState, EditSource, RestoreEligibility};

use super::session::resolve_id;
use super::testing::{SESSION, Sandbox, record_commit};
use super::{BeforeSummary, ChangeItem, RecoveryCommand, StatusNow, run_recovery};
use crate::agent::session_recovery_attach::attach_recovery;
use crate::agent::tools::DatabaseTools;
use crate::commands::recovery::sha256_hex;
use crate::interactive::session::lifecycle::recovery::RecoveryHandle;
use crate::interactive::session::universe::{SessionComposition, SessionUniverse};

const NOW: i64 = 1_700_000_000_000;

/// The retention text `/recovery on` must say, byte for byte, on every
/// surface. Written out here so production can never drift from it.
const ON_TEXT: &str = "Recovery is on. From the next file edit, workspace_write and workspace_edit keep the file's previous contents so you can undo an edit. The copies are private to you (owner-only files under the saya sessions directory, outside your project), limited to 128 edits, 64 MiB and 8 MiB per file, and are deleted with the session (/sessions delete). Recovery covers direct file-tool edits only — never commands, downloads, scratch tables or databases — and files that look like credentials or secrets are edited without a copy. Edits made before this point were not recorded. Use /recovery list to see them and /recovery restore <id> to undo one.";

fn handle_for(sandbox: &Sandbox) -> RecoveryHandle {
    RecoveryHandle::new(&sandbox.outer.join("sessions"), SESSION)
}

include!("session_tests/status_and_flags.rs");

fn item(id: &str) -> ChangeItem {
    ChangeItem {
        id: id.to_owned(),
        path: "notes.txt".to_owned(),
        source: EditSource::WorkspaceEdit,
        state: CheckpointState::Committed,
        before: BeforeSummary::Absent,
        after: None,
        created_unix_ms: NOW,
        restore: StatusNow::Known(RestoreEligibility::Eligible),
    }
}

include!("session_tests/record_id_resolution.rs");

include!("session_tests/session_surface.rs");

async fn write_through(universe: &SessionUniverse, args: serde_json::Value) -> serde_json::Value {
    let tools = Arc::new(attached_tools(universe));
    universe
        .executor(Arc::clone(&tools), &CancellationToken::new(), false)
        .execute("workspace_write", args)
        .await
        .expect("the write succeeds")
}

/// The per-turn shape: fresh tools over the session's workspace, then the
/// attach — exactly what `run_prompt` builds per turn.
fn attached_tools(universe: &SessionUniverse) -> DatabaseTools {
    attach_recovery(
        DatabaseTools::new(None, 100, true).with_workspace(universe.workspace()),
        Some(universe),
    )
}

fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-recovery-session-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("the temp root");
    root
}

/// A minimal resolved runtime for the composed universe: no profile, no
/// secrets, temp-rooted investigations root — the shape the attach tests
/// build, inlined here because the TUI's test-support module is private
/// to its tree.
fn test_runtime() -> crate::config::runtime::RuntimeConfig {
    use saya_config::{
        AiProvider, ColorChoice, ConnectionsFile, MemoryMode, OutputFormat, ResolvedAi,
        ResolvedConfig, ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs,
        ResolvedJobs, ResolvedMemory, ResolvedRunnerJobs, ThemeChoice,
    };
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
        connections: ConnectionsFile::default(),
        config_path: None,
        connections_path: None,
        cache_scope: temp_dir("recovery-cache"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}
