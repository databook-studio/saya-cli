//! Chain tests for the TUI dispatch chain (P0): every registered slash
//! command must survive the real `dispatch` without panicking, and the
//! file-writing commands must write through it. The chain used to panic
//! whenever a helper consumed an action and then reported "not mine"
//! (`.expect("helper passes the action through")`); these tests prove no
//! registered command can trip that again.

use super::Dispatch;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::application::tests_support::{idle_app, unused_runtime};
use crate::interactive::tui::capture::CapturedResult;
use crate::interactive::tui::transcript::{BlockKind, Transcript};
use crate::interactive::tui::types::{App, LastQuery};
use crate::render::RenderFormat;
use crate::slash::registry::KNOWN_COMMANDS;
use saya_agent::ApprovalPolicy;
use saya_store::FsSessionStore;
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, SqlDialect,
};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

const STARTED_UNIX_MS: i64 = 1_700_000_000_000;

/// Removes the throwaway root when the fixture drops. Declared after the
/// session runtime in the fixture so the lock file is released before the
/// directory goes.
struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One fixture: the exact pieces the real `dispatch` call takes (the shape
/// `loop_tick::pending` passes), backed by a throwaway root.
struct ChainFixture {
    app: App,
    state: SessionState,
    session: SessionRuntime,
    store: FsSessionStore,
    runtime: RuntimeConfig,
    root: RootGuard,
}

impl ChainFixture {
    fn build(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-chain-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("chain test root");
        let mut runtime = unused_runtime();
        // The composed investigations root: every investigation operation the
        // dispatch runs writes under this fixture's own root, never the
        // machine's real data directory.
        runtime.investigations_root = root.join("investigations");
        runtime.connections.profiles.insert(
            "demo".to_string(),
            saya_types::DatabaseProfile::Sqlite {
                path: root.join("demo.sqlite3").display().to_string(),
                read_only: true,
            },
        );
        let session = SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            "chain-test",
            ApprovalPolicy::Ask,
            &root.join("sessions"),
        )
        .expect("the chain test session acquires");
        Self {
            app: idle_app(),
            state: SessionState::new("chain-test", None, "model"),
            session,
            store: FsSessionStore::new(root.join("session-store")),
            runtime,
            root: RootGuard(root),
        }
    }
}

/// The real `dispatch`, called with the fixture's pieces exactly as the
/// live loop does.
fn dispatch_line(line: &str, fx: &mut ChainFixture) -> Dispatch {
    super::dispatch(
        line,
        &mut fx.app.transcript,
        &fx.app.profiles,
        &mut fx.state,
        &fx.runtime,
        &fx.store,
        &fx.app.state_db,
        RenderFormat::Text,
        &mut fx.app.last_query,
        &fx.app.captured,
        fx.app.agent_captures.gap,
        &mut fx.session,
    )
}

/// One plausible line per registered command name; file destinations live
/// under the fixture's root. A new registry entry without a line here fails
/// the test naming the entry to fill in.
fn representative_line(name: &str, root: &std::path::Path) -> String {
    let path = |file: &str| root.join(file).display().to_string();
    let line = match name {
        "connect" => "/connect demo",
        "connections" => "/connections",
        "include" => "/include demo",
        "exclude" => "/exclude demo",
        "provider" => "/provider",
        "model" => "/model",
        "privacy" => "/privacy",
        "approvals" => "/approvals",
        "mode" => "/mode",
        "schema" => "/schema",
        "sql" => "/sql SELECT 1",
        "export" => return format!("/export --snapshot {}", path("export.csv")),
        "report" => return format!("/report {}", path("report.md")),
        "investigation" => "/investigation save chain-sentinel --sql SELECT 1 --connection demo",
        "investigations" => "/investigations",
        "chart" => return format!("/chart line {}", path("chart.png")),
        "explain" => "/explain SELECT 1",
        "clear" => "/clear",
        "compact" => "/compact",
        "history" => "/history",
        "sessions" => "/sessions",
        "resume" => "/resume 20260101-000000-abc",
        "columns" => "/columns",
        "contracts" => "/contracts",
        "contract" => "/contract",
        "remember" => "/remember users description orders-table",
        "forget" => "/forget 0000",
        "queue" => "/queue",
        "confirm" => "/confirm 0000",
        "reject" => "/reject 0000",
        "approve-all" => "/approve-all",
        "run" => "/run survey the data",
        "runs" => "/runs",
        "allow" => "/allow none",
        "grants" => "/grants",
        "doctor" => "/doctor",
        "usage" => "/usage",
        "workspace" => "/workspace",
        "thinking" => "/thinking",
        "tasks" => "/tasks",
        "help" => "/help",
        "exit" => "/exit",
        "quit" => "/quit",
        other => panic!("no representative line for /{other}: add one to chain_tests"),
    };
    line.to_string()
}

/// Every registered slash command survives the real dispatch: the chain
/// must never panic on an unowned action. Each command gets a fresh
/// fixture and its representative line; the assertion is that `dispatch`
/// returns — whatever outcome it picks.
#[test]
fn every_registered_slash_command_survives_dispatch() {
    let root = std::env::temp_dir().join(format!(
        "saya-chain-lines-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("chain test lines root");
    let mut failed: Vec<String> = Vec::new();
    for name in KNOWN_COMMANDS {
        let line = representative_line(name, &root);
        let tag = name.replace('-', "_");
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let mut fx = ChainFixture::build(&tag);
            dispatch_line(&line, &mut fx)
        }));
        if let Err(payload) = outcome {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("<non-string panic>");
            failed.push(format!("/{name} panicked ({line}): {message}"));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        failed.is_empty(),
        "{} registered command(s) panicked in dispatch:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

fn sample_result() -> QueryResult {
    QueryResult {
        columns: vec!["id".to_string(), "name".to_string()],
        rows: vec![
            serde_json::json!([1, "sentinel-alice"]),
            serde_json::json!([2, "bob"]),
        ],
        row_count: 2,
        truncated: false,
        executed_sql: "SELECT id, name FROM users".to_string(),
    }
}

fn captured_of(result: QueryResult) -> Option<CapturedResult> {
    let evidence = ExecutionEvidence::for_result(
        &result,
        ExecutionEvidenceArgs {
            execution_id: "xabc-1".to_string(),
            connection_label: "demo".to_string(),
            connection_identity: None,
            dialect: SqlDialect::Sqlite,
            max_rows: 100,
            started_unix_ms: STARTED_UNIX_MS,
            finished_unix_ms: STARTED_UNIX_MS + 2_000,
            source: EvidenceSource::DirectSql,
        },
    );
    Some(CapturedResult { result, evidence })
}

fn last_block(transcript: &Transcript, kind: BlockKind) -> Option<String> {
    transcript
        .blocks()
        .iter()
        .rev()
        .find(|block| block.kind == kind)
        .map(|block| block.text.clone())
}

/// `/sql` (its task returned untouched) then `/export --snapshot`: the
/// snapshot writes the captured result through the real dispatch, with no
/// query dispatched at all.
#[test]
fn export_snapshot_after_sql_writes_the_file_through_dispatch() {
    let mut fx = ChainFixture::build("export-snapshot");
    fx.app.captured = captured_of(sample_result());
    let outcome = dispatch_line("/sql SELECT 1", &mut fx);
    assert!(
        matches!(outcome, Dispatch::SqlTask(_)),
        "/sql dispatches its task"
    );
    let path = fx.root.0.join("snapshot.csv");
    let outcome = dispatch_line(&format!("/export --snapshot {}", path.display()), &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the snapshot export is handled inline"
    );
    let written = std::fs::read_to_string(&path).expect("the snapshot file was written");
    assert!(
        written.contains("sentinel-alice"),
        "the rows come from the capture: {written}"
    );
    let msg = last_block(&fx.app.transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("Exported") && msg.contains("snapshot exec"),
        "the success names the capture: {msg}"
    );
}

/// `/investigation save` with neither --sql nor --file fills from the last
/// query through the real dispatch, saves into the state db, and
/// `/investigations` lists it back through the same chain.
#[test]
fn investigation_save_through_dispatch_saves() {
    let mut fx = ChainFixture::build("investigation-save");
    fx.app.last_query = Some(LastQuery {
        sql: "SELECT 1".to_string(),
        connection: Some("demo".to_string()),
    });
    let outcome = dispatch_line("/investigation save chain-sentinel", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "an inline save reports handled"
    );
    let msg = last_block(&fx.app.transcript, BlockKind::System).expect("the save said something");
    assert!(
        msg.contains("Saved exactly as shown"),
        "the save succeeded: {msg}"
    );
    let outcome = dispatch_line("/investigations", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the listing reports handled"
    );
    let list = last_block(&fx.app.transcript, BlockKind::System).expect("the listing is said");
    assert!(
        list.contains("chain-sentinel"),
        "the saved investigation is listed: {list}"
    );
}

/// The data-dir guard (T2): the investigation fixtures must never touch the
/// machine's real data directory. The default root — computed the production
/// way (`SAYA_INVESTIGATIONS_DIR` when set, else beside the state DB) — must
/// gain no files while the investigation fixtures run through the real
/// dispatch, every fixture's composed root must sit under the temp dir, and
/// the save must have landed in that composed root (a positive control: the
/// fixtures really write, just somewhere private).
#[test]
fn tests_never_touch_the_real_data_dir() {
    let default_root = crate::config::runtime::investigations_root_from(
        std::env::var_os("SAYA_INVESTIGATIONS_DIR").as_deref(),
        &crate::state_path::state_db_path(),
    );
    let before = document_count(&default_root);

    // The chain fixture's composed root, plus the roots the two shared App
    // fixtures compose: all of them must be temp-rooted.
    let fixture_roots = [
        (
            "chain fixture",
            ChainFixture::build("data-dir-guard")
                .runtime
                .investigations_root,
        ),
        ("idle app", idle_app().runtime.investigations_root.clone()),
        ("unused runtime", unused_runtime().investigations_root),
    ];
    for (name, root) in &fixture_roots {
        assert!(
            root.starts_with(std::env::temp_dir()),
            "the {name} composed its investigations root at {root:?}, outside the temp dir"
        );
    }

    let mut fx = ChainFixture::build("data-dir-guard-writes");
    fx.app.last_query = Some(LastQuery {
        sql: "SELECT 1".to_string(),
        connection: Some("demo".to_string()),
    });
    let outcome = dispatch_line("/investigation save data-dir-guard-sentinel", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the save dispatches inline"
    );
    let outcome = dispatch_line("/investigations", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the listing dispatches inline"
    );
    let outcome = dispatch_line("/investigation run no-such-ffffffff", &mut fx);
    assert!(
        matches!(outcome, Dispatch::ReplayTask(_)),
        "the run dispatches its task without running it"
    );
    assert!(
        document_count(&fx.runtime.investigations_root) > 0,
        "the save really wrote into the fixture's composed root"
    );

    let after = document_count(&default_root);
    assert_eq!(
        before, after,
        "the investigations root {default_root:?} gained files during the fixtures"
    );
}

/// Entries in a directory, or 0 when it does not exist — a read-only count,
/// never a creation.
fn document_count(root: &std::path::Path) -> usize {
    std::fs::read_dir(root)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// `/report` writes the Markdown report from the capture through the real
/// dispatch: no query, no task, the file on disk.
#[test]
fn report_through_dispatch_writes() {
    let mut fx = ChainFixture::build("report");
    fx.app.captured = captured_of(sample_result());
    let path = fx.root.0.join("report.md");
    let outcome = dispatch_line(&format!("/report --rows 2 {}", path.display()), &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the report is handled inline"
    );
    let written = std::fs::read_to_string(&path).expect("the report was written");
    assert!(
        written.contains("sentinel-alice"),
        "the rows come from the capture: {written}"
    );
    let msg = last_block(&fx.app.transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("Wrote report to") && msg.contains("(2 rows included)"),
        "the success message counts the rows: {msg}"
    );
}
