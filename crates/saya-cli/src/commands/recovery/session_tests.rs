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
use crate::interactive::session_recovery::RecoveryHandle;
use crate::interactive::session_universe::{SessionComposition, SessionUniverse};

const NOW: i64 = 1_700_000_000_000;

/// The retention text `/recovery on` must say, byte for byte, on every
/// surface. Written out here so production can never drift from it.
const ON_TEXT: &str = "Recovery is on. From the next file edit, workspace_write and workspace_edit keep the file's previous contents so you can undo an edit. The copies are private to you (owner-only files under the saya sessions directory, outside your project), limited to 128 edits, 64 MiB and 8 MiB per file, and are deleted with the session (/sessions delete). Recovery covers direct file-tool edits only — never commands, downloads, scratch tables or databases — and files that look like credentials or secrets are edited without a copy. Edits made before this point were not recorded. Use /recovery list to see them and /recovery restore <id> to undo one.";

fn handle_for(sandbox: &Sandbox) -> RecoveryHandle {
    RecoveryHandle::new(&sandbox.outer.join("sessions"), SESSION)
}

#[test]
fn status_reports_the_flag_and_the_quota_usage() {
    let sandbox = Sandbox::new("recovery-status");
    let handle = handle_for(&sandbox);
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert_eq!(
        text, "Recovery is off. 0 recorded edit(s) this session (0 KiB of 64 MiB, 0 of 128).",
        "{text}"
    );

    // Two committed edits through the real store lifecycle: the counts and
    // the quota line follow, and the off-state hint says they stay usable.
    record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(b"v1\n"),
        b"v2\n",
        NOW,
    );
    record_commit(
        &sandbox,
        "other.txt",
        EditSource::WorkspaceWrite,
        Some(b"a\n"),
        b"b\n",
        NOW,
    );
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert!(
        text.starts_with("Recovery is off. 2 recorded edit(s) this session ("),
        "{text}"
    );
    assert!(text.contains(" of 64 MiB, 2 of 128)."), "{text}");
    assert!(
        text.contains(" Recorded edits stay available: /recovery list."),
        "{text}"
    );

    // On with the same records: the hint belongs to the off state only.
    handle.set_enabled(true);
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert!(
        text.starts_with("Recovery is on. 2 recorded edit(s)"),
        "{text}"
    );
    assert!(!text.contains("stay available"), "{text}");
}

#[test]
fn status_without_a_workspace_says_so() {
    let sandbox = Sandbox::new("recovery-status-unbound");
    let handle = handle_for(&sandbox);
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, None, NOW);
    assert!(ok, "{text}");
    assert!(
        text.contains("Recovery is off. 0 recorded edit(s)"),
        "{text}"
    );
    assert!(text.ends_with(" No workspace is bound."), "{text}");
}

#[test]
fn on_pins_the_retention_text_and_flips_the_session_flag() {
    let sandbox = Sandbox::new("recovery-on");
    let handle = handle_for(&sandbox);
    assert!(
        !handle.is_enabled(),
        "recovery is opt-in: off at acquisition"
    );
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert_eq!(text, ON_TEXT, "the retention text is said verbatim");
    assert!(handle.is_enabled(), "the session's own flag flipped on");
}

#[test]
fn on_twice_says_already_on() {
    let sandbox = Sandbox::new("recovery-on-twice");
    let handle = handle_for(&sandbox);
    let (ok, _) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), NOW);
    assert!(ok);
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert_eq!(text, "Recovery is already on.", "{text}");
    assert!(handle.is_enabled(), "the flag stayed on");
}

#[test]
fn on_without_a_workspace_refuses_and_the_flag_stays_off() {
    let sandbox = Sandbox::new("recovery-on-unbound");
    let handle = handle_for(&sandbox);
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, None, NOW);
    assert!(!ok, "{text}");
    assert_eq!(
        text,
        "No workspace is bound, so there are no file edits to record. Launch inside a \
         git worktree or pass --workspace <dir>.",
        "{text}"
    );
    assert!(!handle.is_enabled(), "the flag stayed off");
}

#[test]
fn off_flips_the_flag_and_keeps_what_was_recorded() {
    let sandbox = Sandbox::new("recovery-off");
    let handle = handle_for(&sandbox);
    let (ok, _) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), NOW);
    assert!(ok);
    record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(b"v1\n"),
        b"v2\n",
        NOW,
    );
    let (ok, text) = run_recovery(RecoveryCommand::Off, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert_eq!(text, "Recovery is off.", "{text}");
    assert!(!handle.is_enabled(), "the flag flipped off");
    // What was recorded before the off stays available.
    let (ok, text) = run_recovery(RecoveryCommand::List, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert!(
        text.contains("notes.txt"),
        "the recorded edit is listed: {text}"
    );
}

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

#[test]
fn the_full_id_and_a_unique_prefix_resolve_to_the_one_edit() {
    let full = "ck-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let items = [item(full)];
    assert_eq!(resolve_id(&items, full).expect("full id"), full);
    assert_eq!(
        resolve_id(&items, "ck-aaaaaa").expect("unique 6-char prefix"),
        full,
        "the unique prefix resolves to the one edit it names"
    );
    assert_eq!(
        resolve_id(&items, "ck-aaaaaaaaaaaaaaaa").expect("longer prefix"),
        full
    );
}

#[test]
fn an_ambiguous_prefix_names_the_problem() {
    let items = [
        item("ck-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        item("ck-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab"),
    ];
    let error = resolve_id(&items, "ck-aaaaaa").expect_err("two edits share the prefix");
    assert!(
        error.contains("matches 2 recorded edits"),
        "the ambiguity is named: {error}"
    );
}

#[test]
fn a_too_short_prefix_is_refused() {
    let items = [item("ck-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")];
    let error = resolve_id(&items, "ck-ab").expect_err("2 characters is too short");
    assert!(
        error.contains("too short"),
        "the short prefix is named: {error}"
    );
}

#[test]
fn an_unknown_or_malformed_id_is_refused_without_reaching_the_store() {
    let items = [item("ck-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")];
    let error = resolve_id(&items, "ck-ffffff").expect_err("no such edit");
    assert!(
        error.contains("matches no recorded edit"),
        "the unknown prefix is named: {error}"
    );
    let error = resolve_id(&items, "zzzzzz").expect_err("no ck- prefix");
    assert!(
        error.contains("ck-"),
        "the malformed id names the id shape: {error}"
    );
}

#[tokio::test]
async fn the_session_surface_records_previews_and_restores_real_edits() {
    let root = temp_dir("recovery-e2e");
    let project = root.join("project");
    fs::create_dir_all(project.join(".git")).expect("the worktree directory");
    let state_dir = root.join("state");
    fs::create_dir_all(&state_dir).expect("the state directory");
    let sessions_root = root.join("sessions");
    let runtime = test_runtime();
    let handle = RecoveryHandle::new(&sessions_root, "e2e-recovery");
    let universe = SessionUniverse::compose_with_launch_and_path(SessionComposition {
        runtime: &runtime,
        explicit: Some(&project),
        pinned_root: None,
        walk_when_unpinned: true,
        cwd: &project,
        state_dir: &state_dir,
        launch: None,
        path: Some("e2e-path".to_owned()),
        scratch: None,
        recovery: Some(handle.clone()),
    })
    .expect("the session composes over the temp worktree");
    let ws = universe.workspace();
    assert!(ws.is_some(), "the explicit root binds a workspace");

    // Flag OFF: the create records nothing and the listing says so.
    let created = write_through(
        &universe,
        serde_json::json!({"path": "notes.txt", "content": "first line\n"}),
    )
    .await;
    assert!(
        created.get("recovery").is_none(),
        "flag off: the create records nothing"
    );
    let (ok, text) = run_recovery(RecoveryCommand::List, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert!(text.contains("No recorded edits"), "{text}");

    // Turning it on says the retention text verbatim.
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert_eq!(text, ON_TEXT, "the retention text is said verbatim");
    assert!(handle.is_enabled(), "the session's own flag flipped on");

    // A guarded replace through the executor records exactly one checkpoint.
    let digest = created["digest"]
        .as_str()
        .expect("the write names its digest")
        .to_owned();
    let replaced = write_through(
        &universe,
        serde_json::json!({
            "path": "notes.txt",
            "content": "second line\n",
            "expected_digest": digest,
        }),
    )
    .await;
    assert_eq!(replaced["recovery"], "checkpoint recorded");

    // The listing shows one committed, restorable edit.
    let (ok, text) = run_recovery(RecoveryCommand::List, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert!(
        text.contains("Recorded edits this session (1 of 128"),
        "{text}"
    );
    assert!(text.contains("workspace_write committed"), "{text}");
    assert!(
        text.contains("restorable now: the file still matches the recorded edit"),
        "{text}"
    );
    let full_id = text
        .split_whitespace()
        .find(|token| token.starts_with("ck-"))
        .expect("the listing carries the checkpoint id")
        .to_owned();
    let prefix = format!("ck-{}", &full_id[3..9]);

    // The diff preview shows the exact changed line.
    let (ok, text) = run_recovery(
        RecoveryCommand::Diff(prefix.clone()),
        &handle,
        ws.as_deref(),
        NOW,
    );
    assert!(ok, "{text}");
    assert!(
        text.contains("first line"),
        "the removed line is shown: {text}"
    );
    assert!(
        text.contains("second line"),
        "the added line is shown: {text}"
    );

    // The restore puts the exact before bytes back and appends the reminder.
    let (ok, text) = run_recovery(
        RecoveryCommand::Restore(prefix.clone()),
        &handle,
        ws.as_deref(),
        NOW,
    );
    assert!(ok, "{text}");
    assert!(
        text.contains("Restored notes.txt to its state before the edit"),
        "{text}"
    );
    assert!(
        text.contains("Restoring a file does not undo commands, downloads or database effects."),
        "the reminder line is appended: {text}"
    );
    assert_eq!(
        fs::read(project.join("notes.txt")).expect("the restored file"),
        b"first line\n",
        "the file holds the exact pre-edit bytes"
    );

    // A second restore of the consumed checkpoint: nothing to restore.
    let (ok, text) = run_recovery(
        RecoveryCommand::Restore(prefix),
        &handle,
        ws.as_deref(),
        NOW,
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("no recorded edit"),
        "the consumed checkpoint is named: {text}"
    );

    // Off: recording stops; an edit made now records nothing.
    let (ok, text) = run_recovery(RecoveryCommand::Off, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert_eq!(text, "Recovery is off.", "{text}");
    assert!(!handle.is_enabled(), "the flag flipped off");
    let replaced = write_through(
        &universe,
        serde_json::json!({
            "path": "notes.txt",
            "content": "third line\n",
            "expected_digest": sha256_hex(b"first line\n"),
        }),
    )
    .await;
    assert!(
        replaced.get("recovery").is_none(),
        "flag off: the edit records nothing"
    );
    let (ok, text) = run_recovery(RecoveryCommand::List, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert!(text.contains("No recorded edits"), "{text}");

    // On again, record one edit, then off: the recorded edit survives and a
    // restore still works with the flag off — restoring is the user's own act.
    let (ok, _) = run_recovery(RecoveryCommand::On, &handle, ws.as_deref(), NOW);
    assert!(ok);
    let replaced = write_through(
        &universe,
        serde_json::json!({
            "path": "notes.txt",
            "content": "fourth line\n",
            "expected_digest": sha256_hex(b"third line\n"),
        }),
    )
    .await;
    assert_eq!(replaced["recovery"], "checkpoint recorded");
    let (ok, text) = run_recovery(RecoveryCommand::Off, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert!(
        text.starts_with("Recovery is off. 1 recorded edit(s)"),
        "{text}"
    );
    assert!(
        text.contains(" Recorded edits stay available: /recovery list."),
        "{text}"
    );
    let (ok, list) = run_recovery(RecoveryCommand::List, &handle, ws.as_deref(), NOW);
    assert!(ok, "{list}");
    let full_id = list
        .split_whitespace()
        .find(|token| token.starts_with("ck-"))
        .expect("the listing carries the checkpoint id")
        .to_owned();
    let (ok, text) = run_recovery(
        RecoveryCommand::Restore(full_id),
        &handle,
        ws.as_deref(),
        NOW,
    );
    assert!(ok, "restore works with the flag off: {text}");
    assert_eq!(
        fs::read(project.join("notes.txt")).expect("the restored file"),
        b"third line\n",
        "the file holds the exact pre-edit bytes"
    );

    // A file edited again by hand after the edit was recorded: the restore
    // refuses and the file is untouched.
    let (ok, _) = run_recovery(RecoveryCommand::On, &handle, ws.as_deref(), NOW);
    assert!(ok);
    let replaced = write_through(
        &universe,
        serde_json::json!({
            "path": "notes.txt",
            "content": "sixth line\n",
            "expected_digest": sha256_hex(b"third line\n"),
        }),
    )
    .await;
    assert_eq!(replaced["recovery"], "checkpoint recorded");
    let (ok, list) = run_recovery(RecoveryCommand::List, &handle, ws.as_deref(), NOW);
    assert!(ok, "{list}");
    let full_id = list
        .split_whitespace()
        .find(|token| token.starts_with("ck-"))
        .expect("the listing carries the checkpoint id")
        .to_owned();
    fs::write(project.join("notes.txt"), b"hand edit\n").expect("the hand edit");
    let (ok, text) = run_recovery(
        RecoveryCommand::Restore(full_id),
        &handle,
        ws.as_deref(),
        NOW,
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("changed after that edit was recorded"),
        "the newer-work refusal is named: {text}"
    );
    assert_eq!(
        fs::read(project.join("notes.txt")).expect("the untouched file"),
        b"hand edit\n",
        "the refused restore left the file untouched"
    );

    let _ = fs::remove_dir_all(&root);
}

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
