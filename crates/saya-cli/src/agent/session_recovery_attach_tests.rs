//! The attach seam's wiring tests: a real acquired session with a bound
//! workspace carries one handle — the acquired id, flag off — and the
//! per-turn tools built through `attach_recovery` capture only when the
//! session's own flag is on. An unbound universe exposes no handle and
//! attaches nothing; the one-shot `ask` path (`None`) attaches nothing.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolExecutor};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_types::CheckpointState;

use super::attach_recovery;
use crate::agent::tools::DatabaseTools;
use crate::interactive::session_recovery::RecoveryHandle;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_universe::{SessionComposition, SessionUniverse};

fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-attach-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn test_runtime() -> crate::config::runtime::RuntimeConfig {
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
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-session-attach"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// The per-turn shape: fresh tools over the session's workspace, then the
/// attach. Exactly what `run_prompt` builds per turn.
fn attached_tools(universe: &SessionUniverse) -> DatabaseTools {
    attach_recovery(
        DatabaseTools::new(None, 100, true).with_workspace(universe.workspace()),
        Some(universe),
    )
}

/// The real wiring: the flag OFF leaves the create byte-identical to today
/// — no `recovery` key, no checkpoint data — and flipping the session's
/// flag ON makes a guarded replace commit exactly one checkpoint whose
/// before-image is the pre-edit bytes, under the acquired session id.
#[tokio::test]
async fn flag_off_matches_today_and_flag_on_records_the_replace() {
    let root = temp_dir("wiring");
    let project = temp_dir("wiring-project");
    let sessions_root = root.join("sessions");
    let runtime = test_runtime();
    let session = SessionRuntime::acquire(
        &runtime,
        Some(&project),
        true,
        None,
        "attach-session",
        ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("the session acquires with an explicit workspace");

    // One handle per acquisition: the acquired id, flag off, and the
    // universe exposes it only because a workspace is bound.
    let handle = session.recovery().clone();
    assert_eq!(handle.parts().1, "attach-session", "the acquired id");
    assert!(
        !handle.is_enabled(),
        "recovery is opt-in: OFF at acquisition"
    );
    let universe = session.universe();
    let exposed = universe
        .recovery_handle()
        .expect("a bound workspace exposes the handle");
    assert!(
        Arc::ptr_eq(&exposed.parts().2, &handle.parts().2),
        "the universe carries the same flag cell"
    );

    // Flag OFF: the create through the session executor is byte-identical
    // to today — no `recovery` key, no checkpoints directory.
    let created = Arc::new(attached_tools(&universe));
    let result = universe
        .executor(Arc::clone(&created), &CancellationToken::new(), false)
        .execute(
            "workspace_write",
            serde_json::json!({"path": "notes.txt", "content": "first line\n"}),
        )
        .await
        .expect("the create succeeds");
    assert!(
        result.get("recovery").is_none(),
        "flag off: no recovery field"
    );
    assert!(
        !sessions_root
            .join("attach-session")
            .join("checkpoints")
            .exists(),
        "flag off: no checkpoint data is stored"
    );
    let digest = result["digest"]
        .as_str()
        .expect("the create names the digest")
        .to_owned();

    // Flag ON — the session's own cell, what the toggle will flip: a
    // guarded replace records the pre-edit bytes exactly, once.
    handle.set_enabled(true);
    let replaced = Arc::new(attached_tools(&universe));
    let result = universe
        .executor(Arc::clone(&replaced), &CancellationToken::new(), false)
        .execute(
            "workspace_write",
            serde_json::json!({
                "path": "notes.txt",
                "content": "second line\n",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the guarded replace succeeds");
    assert_eq!(result["recovery"], "checkpoint recorded");

    let (store, session_id, _) = handle.parts();
    assert_eq!(session_id, "attach-session");
    let records = store
        .list_checkpoints("attach-session")
        .expect("the listing reads back");
    assert_eq!(records.len(), 1, "exactly the replace was recorded");
    assert_eq!(records[0].state, CheckpointState::Committed);
    let image = store
        .read_before_image("attach-session", records[0].id.as_str())
        .expect("the before-image reads back");
    assert_eq!(
        image, b"first line\n",
        "the stored image is the pre-edit bytes"
    );
    drop(session);
    let _ = fs::remove_dir_all(&root);
}

/// A universe with a handle but NO workspace exposes none of it: no
/// workspace, no file tools, no recorder — even with the flag on, the
/// attach is a no-op and the write refuses exactly as unattached.
#[tokio::test]
async fn an_unbound_universe_exposes_no_handle_and_attaches_nothing() {
    let root = temp_dir("unbound");
    let plain = temp_dir("unbound-plain");
    let state = temp_dir("unbound-state");
    let runtime = test_runtime();
    let handle = RecoveryHandle::new(&root.join("sessions"), "unbound-session");
    let universe = SessionUniverse::compose_with_launch_and_path(SessionComposition {
        runtime: &runtime,
        explicit: None,
        pinned_root: None,
        walk_when_unpinned: true,
        cwd: &plain,
        state_dir: &state,
        launch: None,
        path: Some("test-path".to_owned()),
        scratch: None,
        recovery: Some(handle.clone()),
    })
    .expect("an unbound session composes");
    assert!(universe.root().is_none(), "outside a worktree, no root");
    assert!(
        universe.recovery_handle().is_none(),
        "no workspace means no handle, whatever the flag says"
    );

    handle.set_enabled(true);
    let args = serde_json::json!({"path": "x.txt", "content": "y"});
    let unattached = DatabaseTools::new(None, 100, true)
        .execute("workspace_write", args.clone())
        .await
        .expect_err("no workspace, no write");
    let attached = attach_recovery(DatabaseTools::new(None, 100, true), Some(&universe))
        .execute("workspace_write", args)
        .await
        .expect_err("the attach changed nothing without a workspace");
    assert_eq!(
        unattached, attached,
        "the unbound universe's attach is a no-op"
    );
    let _ = fs::remove_dir_all(&root);
}

/// The one-shot `ask` path passes no session: the attach is a no-op and
/// the tools behave exactly as unattached ones.
#[tokio::test]
async fn attaching_without_a_session_is_a_no_op() {
    let args = serde_json::json!({"path": "x.txt", "content": "y"});
    let attached = attach_recovery(DatabaseTools::new(None, 100, true), None)
        .execute("workspace_write", args.clone())
        .await
        .expect_err("no workspace, no write");
    let plain = DatabaseTools::new(None, 100, true)
        .execute("workspace_write", args)
        .await
        .expect_err("no workspace, no write");
    assert_eq!(attached, plain, "the ask path attaches nothing");
}

/// The runtime-level guard: the handle names the acquired id and starts
/// off, and a trust rebind — which recomposes the universe — keeps the
/// SAME flag cell, so the session's toggle survives it.
#[test]
fn the_handle_rides_the_acquired_id_and_a_trust_rebind_keeps_the_flag() {
    let root = temp_dir("rebind");
    let trusted = temp_dir("rebind-trusted");
    let sessions_root = root.join("sessions");
    let runtime = test_runtime();
    let mut session = SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "rebind-session",
        ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("the fresh session acquires");
    let handle = session.recovery().clone();
    assert_eq!(handle.parts().1, "rebind-session", "the acquired id");
    assert!(!handle.is_enabled(), "OFF at acquisition");
    handle.set_enabled(true);

    session
        .bind_trusted(&runtime, &trusted, None)
        .expect("the trust answer rebinds");
    let rebound = session
        .universe()
        .recovery_handle()
        .expect("the trusted workspace binds");
    assert!(rebound.is_enabled(), "the flag rides the rebind");
    assert_eq!(rebound.parts().1, "rebind-session", "the same id");
    assert!(
        Arc::ptr_eq(&rebound.parts().2, &handle.parts().2),
        "one flag cell per session, across the rebind"
    );
    drop(session);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&trusted);
}
