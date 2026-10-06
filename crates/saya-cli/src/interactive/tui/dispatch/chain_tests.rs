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
use saya_store::{FsSessionStore, SessionStore};
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, SqlDialect,
};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

const STARTED_UNIX_MS: i64 = 1_700_000_000_000;

/// Body text written into the fixture's AGENTS.md; `/context` must name the
/// file and never this text.
const CONTEXT_SENTINEL: &str = "SENTINEL-INSTRUCTION-BODY";

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
    /// The launch options the loop's `SetupInputs` carries — `/reload` reads
    /// these for its same-launch-options re-read. Default everywhere except
    /// the reload tests, which point `--config` at a fixture file.
    launch_options: crate::cli::GlobalOptions,
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
            launch_options: crate::cli::GlobalOptions::default(),
            root: RootGuard(root),
        }
    }

    /// The unbound variant: no explicit root, no pin, and `fresh: false` so
    /// no walk-up runs (a walk-up would bind this repo's own worktree from
    /// the test cwd). The pre-workspace resume shape — nothing binds, which
    /// is what the no-root refusals must be tested against.
    fn build_unbound(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-chain-unbound-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("unbound chain test root");
        let runtime = unused_runtime();
        let session = SessionRuntime::acquire(
            &runtime,
            None,
            false,
            None,
            "chain-test",
            ApprovalPolicy::Ask,
            &root.join("sessions"),
        )
        .expect("the unbound chain test session acquires");
        Self {
            app: idle_app(),
            state: SessionState::new("chain-test", None, "model"),
            session,
            store: FsSessionStore::new(root.join("session-store")),
            runtime,
            launch_options: crate::cli::GlobalOptions::default(),
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
        "notes" => "/notes",
        "rotate" => "/rotate",
        "preferences" => "/preferences",
        "recovery" => "/recovery",
        "resume" => "/resume 20260101-000000-abc",
        "setup" => "/setup",
        "reload" => "/reload",
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
        "context" => "/context",
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

#[test]
fn run_resume_dispatch_keeps_ordinary_and_explicit_retry_shell_commands_distinct() {
    let mut ordinary = ChainFixture::build("resume-ordinary");
    assert!(matches!(
        dispatch_line("/run resume run-ordinary", &mut ordinary),
        Dispatch::Handled
    ));
    let ordinary_hint = last_block(&ordinary.app.transcript, BlockKind::System).unwrap();
    assert!(ordinary_hint.contains("saya run resume run-ordinary`"));
    assert!(!ordinary_hint.contains("--retry-incomplete"));

    let mut retry = ChainFixture::build("resume-retry");
    assert!(matches!(
        dispatch_line("/run resume run-retry --retry-incomplete", &mut retry),
        Dispatch::Handled
    ));
    let retry_hint = last_block(&retry.app.transcript, BlockKind::System).unwrap();
    assert!(retry_hint.contains("saya run resume run-retry --retry-incomplete`"));
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
/// `/investigation list` lists it back through the same chain (bare
/// `/investigations` opens the picker overlay instead, so the text listing
/// is named explicitly here).
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
    let outcome = dispatch_line("/investigation list", &mut fx);
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

/// Bare `/investigations` opens the picker overlay in the TUI: the dispatch
/// returns before the text listing would run, and the overlay opens through
/// the same chain when the loop applies the outcome.
#[test]
fn bare_investigations_opens_the_picker() {
    let mut fx = ChainFixture::build("investigation-picker");
    let outcome = dispatch_line("/investigations", &mut fx);
    assert!(
        matches!(outcome, Dispatch::OpenInvestigationPicker),
        "bare /investigations opens the picker"
    );
    // Any tail keeps the text listing path — the parser handles it.
    let outcome = dispatch_line("/investigations --list", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "--list keeps the text listing"
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
    let outcome = dispatch_line("/investigation list", &mut fx);
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

/// `/context` through the real dispatch: the workspace-root AGENTS.md is
/// named with its whole-file size and truncation flag, and the body text is
/// never printed — the view shows receipts, not contents.
#[test]
fn context_dispatch_lists_the_loaded_source_and_never_its_body() {
    let mut fx = ChainFixture::build("context-lists");
    let body = format!("guidance with {CONTEXT_SENTINEL} inside\n");
    std::fs::write(fx.root.0.join("AGENTS.md"), &body).unwrap();
    assert!(
        matches!(dispatch_line("/context", &mut fx), Dispatch::Handled),
        "/context is handled by the dispatch chain"
    );
    let view =
        last_block(&fx.app.transcript, BlockKind::System).expect("/context pushes a system block");
    assert!(
        view.contains("AGENTS.md"),
        "the loaded source is named: {view}"
    );
    assert!(
        view.contains(&format!("{} B file", body.len())),
        "the whole-file size is shown: {view}"
    );
    assert!(
        view.contains("truncated: no"),
        "the truncation flag is shown: {view}"
    );
    assert!(
        view.contains("never permissions"),
        "the guidance-never-permissions note is stated: {view}"
    );
    assert!(
        !view.contains(CONTEXT_SENTINEL),
        "the source body must never appear: {view}"
    );
}

/// `/setup` through the real dispatch: the ONE new outcome, `Dispatch::RunSetup`
/// — the dispatch never runs the wizard itself (the loop's `tick_pending` owns
/// the handoff under the busy gate), it only routes the request.
#[test]
fn setup_dispatch_returns_run_setup() {
    let mut fx = ChainFixture::build("setup-dispatch");
    assert!(
        matches!(dispatch_line("/setup", &mut fx), Dispatch::RunSetup),
        "the real dispatch of /setup must route the handoff"
    );
}

/// `/setup now` is a parse error, not an argument: an Error block with the
/// pinned wording, and a Handled outcome.
#[test]
fn setup_with_trailing_text_is_a_parse_error_block() {
    let mut fx = ChainFixture::build("setup-parse-error");
    assert!(
        matches!(dispatch_line("/setup now", &mut fx), Dispatch::Handled),
        "the parse error is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("a parse error is an error block");
    assert_eq!(
        error, "setup takes no argument: /setup",
        "the pinned parse-error wording: {error}"
    );
}

/// `/sessions list` through the real dispatch: the shared listing the
/// headless `saya sessions list` prints lands as a System block — this
/// fixture's store is empty, so it says so and names the storage root.
#[test]
fn sessions_list_dispatch_shares_the_headless_listing() {
    let mut fx = ChainFixture::build("sessions-list");
    let outcome = dispatch_line("/sessions list", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the listing is handled inline"
    );
    let listing =
        last_block(&fx.app.transcript, BlockKind::System).expect("/sessions list pushes a block");
    assert!(
        listing.starts_with("No saved sessions.") || listing.contains("Storage root:"),
        "the listing says what the store holds and names the root: {listing}"
    );
}

/// `/sessions delete <the session's own id>` through the real dispatch: the
/// current-session refusal, the D09a words, nothing deleted — the check
/// fires before any filesystem work, so no tombstone is ever written.
#[test]
fn sessions_delete_dispatch_refuses_the_current_session() {
    let mut fx = ChainFixture::build("sessions-delete-current");
    let id = fx.state.id.clone();
    let outcome = dispatch_line(&format!("/sessions delete {id}"), &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("live session") && error.contains("was not deleted"),
        "the refusal is the current-session one, never a silent no-op: {error}"
    );
    assert!(
        !error.contains("pid"),
        "the current-session refusal never reads as a foreign live holder: {error}"
    );
    assert!(
        !fx.root.0.join("lifecycle").exists(),
        "no tombstone was written for the current session"
    );
}

/// `/sessions delete <unknown id>` through the real dispatch: the refusal
/// names the id and nothing is created.
#[test]
fn sessions_delete_dispatch_refuses_an_unknown_id() {
    let mut fx = ChainFixture::build("sessions-delete-nosuch");
    let outcome = dispatch_line("/sessions delete nosuchid-0000", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("no saved session nosuchid-0000"),
        "the refusal names the unknown id: {error}"
    );
    assert!(
        !fx.root.0.join("lifecycle").exists(),
        "an unknown id writes no tombstone"
    );
}

/// `/sessions delete ../x` through the real dispatch: the invalid-id
/// refusal, with nothing resolved on the filesystem.
#[test]
fn sessions_delete_dispatch_refuses_an_invalid_id() {
    let mut fx = ChainFixture::build("sessions-delete-invalid");
    let outcome = dispatch_line("/sessions delete ../x", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("not a valid session id"),
        "the refusal is the invalid-id one: {error}"
    );
}

/// `/preferences list` in a session with no bound root: the shared
/// NoWorktree refusal as an Error block, the exact words the headless CLI
/// prints — never an empty listing, never the launch cwd.
#[test]
fn preferences_dispatch_refuses_without_a_bound_root() {
    let mut fx = ChainFixture::build_unbound("preferences-no-root");
    assert!(
        matches!(
            dispatch_line("/preferences list", &mut fx),
            Dispatch::Handled
        ),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("no project worktree here"),
        "the shared worktree remedy, verbatim: {error}"
    );
}

/// `/preferences bogus` is a parse error block naming the offence and
/// every valid form.
#[test]
fn preferences_dispatch_reports_an_unknown_subcommand() {
    let mut fx = ChainFixture::build("preferences-bogus");
    assert!(
        matches!(
            dispatch_line("/preferences bogus", &mut fx),
            Dispatch::Handled
        ),
        "the parse error is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("a parse error is an error block");
    assert!(
        error.contains("unknown /preferences subcommand: bogus")
            && error.contains("/preferences forget-all"),
        "the error names the offence and the usage: {error}"
    );
}

/// The rooted success path through the real dispatch: `add` confirms one
/// preference and `list` shows it back with its `pp-` id — the same shared
/// operations the headless `saya preferences` runs, scoped to the session's
/// bound root. The fixture's default state db is the empty-path in-memory
/// store (per-connection databases), so this test points the App at a
/// file-backed store under the fixture root — production semantics — before
/// dispatching.
#[test]
fn preferences_add_and_list_through_dispatch() {
    let mut fx = ChainFixture::build("preferences-add-list");
    fx.app.state_db = saya_store::SqliteStateStore::new(fx.root.0.join("state.sqlite3"));
    assert!(
        matches!(
            dispatch_line("/preferences add prefer tabs over spaces", &mut fx),
            Dispatch::Handled
        ),
        "the add is inline"
    );
    let added = last_block(&fx.app.transcript, BlockKind::System).expect("the add says something");
    assert!(added.contains("Added preference pp-"), "{added}");
    assert!(
        matches!(
            dispatch_line("/preferences list", &mut fx),
            Dispatch::Handled
        ),
        "the listing is inline"
    );
    let listing = last_block(&fx.app.transcript, BlockKind::System).expect("the listing is said");
    assert!(listing.contains("prefer tabs over spaces"), "{listing}");
    assert!(
        listing.contains("pp-"),
        "the listing carries the id: {listing}"
    );
}

/// A symlinked AGENTS.md is refused by the loader, and `/context` says so by
/// name and reason — never printing the target's text.
#[cfg(unix)]
#[test]
fn context_dispatch_names_a_skipped_symlink_source() {
    use std::os::unix::fs::symlink;
    let mut fx = ChainFixture::build("context-symlink");
    let outside = fx.root.0.join("outside-agents.md");
    std::fs::write(&outside, "secret rules outside the workspace\n").unwrap();
    symlink(&outside, fx.root.0.join("AGENTS.md")).unwrap();
    assert!(
        matches!(dispatch_line("/context", &mut fx), Dispatch::Handled),
        "/context is handled by the dispatch chain"
    );
    let view =
        last_block(&fx.app.transcript, BlockKind::System).expect("/context pushes a system block");
    assert!(
        view.contains("AGENTS.md"),
        "the skipped source is named: {view}"
    );
    assert!(
        view.contains("symlink refused"),
        "the skip reason is shown: {view}"
    );
    assert!(
        !view.contains("secret rules"),
        "no text from the refused target leaks: {view}"
    );
}

/// Compacts the fixture state through the production compaction path, so
/// `/rotate` has a summary and a continuation record to carry.
fn compact_fixture_state(state: &mut SessionState) {
    use crate::interactive::session_compact::{apply_with_continuation, plan as compact_plan};
    for index in 0..6 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let history = state.provider_history();
    let plan = compact_plan(&state.turns, &history).expect("six turns plan");
    apply_with_continuation(state, &plan, "older turns discussed orders", &|_| None)
        .expect("the summary applies");
}

/// `/rotate` on an uncompacted session: the Error block carries the
/// /compact-first refusal, the session stands, nothing published.
#[test]
fn rotate_dispatch_refuses_an_uncompacted_session() {
    let mut fx = ChainFixture::build("rotate-refusal");
    assert!(
        matches!(dispatch_line("/rotate", &mut fx), Dispatch::Handled),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("run /compact first"),
        "the refusal names the /compact remedy: {error}"
    );
    assert_eq!(fx.state.id, "chain-test", "nothing rotated");
}

/// `/rotate` on a compacted session: the System block carries the exact
/// success sentence, the state is the successor, and the published record
/// links back to the parent.
#[test]
fn rotate_dispatch_rotates_a_compacted_session() {
    let mut fx = ChainFixture::build("rotate-ok");
    compact_fixture_state(&mut fx.state);
    assert!(
        matches!(dispatch_line("/rotate", &mut fx), Dispatch::Handled),
        "the rotation is inline"
    );
    let new = fx.state.id.clone();
    assert_ne!(new, "chain-test", "the state is the successor");
    let message = last_block(&fx.app.transcript, BlockKind::System).expect("the success is said");
    assert_eq!(
        message,
        crate::interactive::session_rotate_run::rotated_message("chain-test", &new),
        "the success sentence is the shared one"
    );
    let successor = crate::interactive::session_resume::block_on(fx.store.load(&new))
        .expect("the successor loads")
        .expect("the successor record exists");
    assert_eq!(
        successor.parent,
        Some(saya_store::ParentLink {
            id: "chain-test".into(),
            rotation_id: new
        }),
        "the store wrote the lineage link"
    );
}

/// A `/rotate` submitted while a request is running WAITS: the queue gate
/// holds the line (the same handling a typed `/resume` relies on — the
/// dispatch chain only ever runs when the app is idle), nothing rotates,
/// and once the request ends the queued line drains through the real
/// dispatch and the rotation runs its refusal.
#[test]
fn a_rotate_submitted_while_busy_waits_for_the_running_request() {
    let mut fx = ChainFixture::build("rotate-busy");
    let before = fx.state.id.clone();
    let (_tx, rx) = std::sync::mpsc::channel();
    fx.app.compact_task = Some(rx);
    fx.app.input.set_text("/rotate");
    fx.app.submit();
    assert_eq!(
        fx.app.pending.as_deref(),
        Some("/rotate"),
        "the busy submit queued the line"
    );
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("Queued")),
        "the queue notice says the line waits: {:?}",
        fx.app
            .transcript
            .blocks()
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(fx.state.id, before, "nothing rotated while busy");

    // The request ends: the queued line drains through the real dispatch.
    fx.app.compact_task = None;
    drain_through_tick_pending(&mut fx);
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the queued rotation ran");
    assert!(
        error.contains("run /compact first"),
        "the uncompacted refusal, once idle: {error}"
    );
    assert_eq!(fx.state.id, before, "the refusal changed nothing");
    wait_for_background_save(&fx.root.0.join("session-store"));
}

/// Gives the loop's background save a moment to land before cleanup, as
/// the resume fixtures do.
fn wait_for_background_save(store_root: &std::path::Path) {
    for _ in 0..200 {
        if std::fs::read_dir(store_root).is_ok_and(|entries| entries.count() > 0) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// `/notes` through the real dispatch with no continuation record: the
/// show is a System block carrying the no-notes words and the remedy.
#[test]
fn notes_dispatch_shows_the_no_notes_words_without_a_record() {
    let mut fx = ChainFixture::build("notes-show");
    assert!(
        matches!(dispatch_line("/notes", &mut fx), Dispatch::Handled),
        "the show is handled inline"
    );
    let shown = last_block(&fx.app.transcript, BlockKind::System).expect("/notes pushes a block");
    assert!(
        shown.contains("No session notes") && shown.contains("/notes set <text>"),
        "the show names the remedy: {shown}"
    );
}

/// `/notes set` without a continuation record: the Error block carries the
/// /compact-first refusal — never a silent no-op, never a created record.
#[test]
fn notes_set_dispatch_refuses_without_a_continuation_record() {
    let mut fx = ChainFixture::build("notes-no-record");
    assert!(
        matches!(
            dispatch_line("/notes set keep these", &mut fx),
            Dispatch::Handled
        ),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("run /compact first"),
        "the refusal names the /compact remedy: {error}"
    );
    assert!(
        fx.state.continuation.is_none(),
        "no record was created by the refusal"
    );
}

/// `/notes set` over a seeded `Current` record: the System block says the
/// notes are stored, the state carries them, and the record still
/// validates.
#[test]
fn notes_set_dispatch_stores_into_a_seeded_record() {
    use saya_types::{CONTINUATION_VERSION, SessionContinuationV1, SourceBoundary};
    let mut fx = ChainFixture::build("notes-seeded");
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 1,
            source_sha256: "c".repeat(64),
            workspace_root: None,
            profile: None,
        },
        narrative: None,
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    };
    fx.state.continuation = Some(saya_store::StoredContinuation::Current(record));
    assert!(
        matches!(
            dispatch_line("/notes set keep these", &mut fx),
            Dispatch::Handled
        ),
        "the store is inline"
    );
    let stored = last_block(&fx.app.transcript, BlockKind::System).expect("the success is said");
    assert!(
        stored.contains("Session notes stored."),
        "the success is the stored words: {stored}"
    );
    let Some(saya_store::StoredContinuation::Current(updated)) = &fx.state.continuation else {
        panic!("the record is still typed: {:?}", fx.state.continuation);
    };
    assert_eq!(updated.user_notes.as_deref(), Some("keep these"));
    updated
        .validate()
        .expect("the updated record still validates");
}

/// `/recovery` through the real dispatch: the status names the off state,
/// `/recovery on` says the retention text and flips the session's own
/// flag, and `/recovery list` renders the (empty) listing.
#[test]
fn recovery_status_on_and_list_dispatch_through_the_chain() {
    let mut fx = ChainFixture::build("recovery-chain");
    assert!(!fx.session.recovery().is_enabled(), "opt-in: off at start");
    assert!(
        matches!(dispatch_line("/recovery", &mut fx), Dispatch::Handled),
        "the status is inline"
    );
    let status = last_block(&fx.app.transcript, BlockKind::System).expect("the status is said");
    assert!(
        status.contains("Recovery is off"),
        "the status names the flag: {status}"
    );

    assert!(
        matches!(dispatch_line("/recovery on", &mut fx), Dispatch::Handled),
        "the toggle is inline"
    );
    let on = last_block(&fx.app.transcript, BlockKind::System).expect("the answer is said");
    assert!(
        on.starts_with("Recovery is on."),
        "the retention text is said: {on}"
    );
    assert!(
        on.contains("never commands, downloads, scratch tables or databases"),
        "the retention text says what it does not cover: {on}"
    );
    assert!(
        fx.session.recovery().is_enabled(),
        "the flag flipped on the session's own handle"
    );

    assert!(
        matches!(dispatch_line("/recovery list", &mut fx), Dispatch::Handled),
        "the listing is inline"
    );
    let list = last_block(&fx.app.transcript, BlockKind::System).expect("the list is said");
    assert!(
        list.contains("No recorded edits"),
        "an empty session says so: {list}"
    );
}

/// `/recovery restore` of an id nothing recorded lands as an Error block
/// through the real dispatch — the unresolved id never reaches the store.
#[test]
fn recovery_restore_of_an_unknown_id_errors_through_the_chain() {
    let mut fx = ChainFixture::build("recovery-restore-unknown");
    assert!(
        matches!(
            dispatch_line("/recovery restore ck-000000", &mut fx),
            Dispatch::Handled
        ),
        "the restore is inline"
    );
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is said");
    assert!(
        error.contains("no recorded edit"),
        "the unknown id is named: {error}"
    );
}

/// The busy rule the TUI holds for restores: while a request is running
/// (here a compaction, the same busy signal `App::is_busy()` covers for a
/// stream, query or replay) a queued `/recovery restore` waits — the
/// restore dispatches only once the request has finished, so it can never
/// overlap a turn that may be writing the same file. Proven through the
/// real loop tick, the same machinery the typed `/resume` and `/compact`
/// pass through.
#[test]
fn a_queued_restore_waits_for_a_running_request() {
    let mut fx = ChainFixture::build("recovery-busy");
    let (_tx, rx) = std::sync::mpsc::channel();
    fx.app.compact_task = Some(rx);
    fx.app.pending = Some("/recovery restore ck-000000".to_string());
    let blocks_before = fx.app.transcript.blocks().len();

    drain_through_tick_pending(&mut fx);
    assert_eq!(
        fx.app.transcript.blocks().len(),
        blocks_before,
        "while busy, the tick dispatches nothing"
    );
    assert!(
        fx.app.pending.is_some(),
        "the restore stays queued while the request runs"
    );

    // The request finished: the queued restore now dispatches, and the
    // unknown id is refused exactly as an immediate dispatch would.
    fx.app.compact_task = None;
    drain_through_tick_pending(&mut fx);
    assert!(fx.app.pending.is_none(), "the queued restore was consumed");
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is said");
    assert!(
        error.contains("no recorded edit"),
        "the restore ran only once idle: {error}"
    );
}

/// The handoff inputs the real `tick_pending` takes: a no-op terminal double
/// (no `/setup` line is drained in these tests, so neither half runs) and
/// default CLI options.
struct NoModes;

impl crate::interactive::tui::terminal_handoff::TerminalModes for NoModes {
    fn release(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn reacquire(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Drains the queued line through the real `tick_pending`, as the live loop
/// does. The fixture's launch options ride `SetupInputs`, exactly as the
/// live loop threads them.
fn drain_through_tick_pending(fx: &mut ChainFixture) {
    let mut modes = NoModes;
    let mut setup = crate::interactive::tui::loop_tick::pending::SetupInputs {
        modes: &mut modes,
        options: &fx.launch_options,
    };
    let _ = crate::interactive::tui::loop_tick::pending::tick_pending(
        &mut fx.app,
        &mut fx.state,
        &fx.runtime,
        &fx.store,
        RenderFormat::Text,
        &mut fx.session,
        &mut setup,
    );
}

/// `/reload` through the real dispatch: the ONE new outcome, `Dispatch::Reload`
/// — the dispatch never reloads itself (the loop's `tick_pending` owns the
/// swap under the busy gate), it only routes the request.
#[test]
fn reload_dispatch_returns_the_reload_outcome() {
    let mut fx = ChainFixture::build("reload-dispatch");
    assert!(
        matches!(dispatch_line("/reload", &mut fx), Dispatch::Reload),
        "the real dispatch of /reload must route the swap"
    );
}

/// `/reload now` is a parse error, not an argument: an Error block with the
/// pinned wording, and a Handled outcome.
#[test]
fn reload_with_trailing_text_is_a_parse_error_block() {
    let mut fx = ChainFixture::build("reload-parse-error");
    assert!(
        matches!(dispatch_line("/reload now", &mut fx), Dispatch::Handled),
        "the parse error is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("a parse error is an error block");
    assert_eq!(
        error, "reload takes no argument: /reload",
        "the pinned parse-error wording: {error}"
    );
}

/// `/reload` drained through the real `tick_pending`, over a fixture whose
/// launch options point `--config` at a fixture file: unchanged files say so;
/// an edited file swaps `app.runtime` and names the key; a broken file leaves
/// an Error block and the OLD runtime handle in place (`Arc::ptr_eq`).
#[test]
fn reload_drained_through_tick_pending_swaps_on_success_and_keeps_the_old_on_failure() {
    use crate::config::reload::{ReloadInputs, build_candidate};
    use std::sync::Arc;
    let mut fx = ChainFixture::build("reload-tick");
    let config = fx.root.0.join("config.toml");
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 100\n",
    )
    .unwrap();
    // The launch options the loop carries: the same `--config` the app's
    // runtime was built from, so a reload re-reads exactly that file. The
    // seed candidate is built through the SAME inputs production reloads
    // with (`from_launch`: real env, cwd "."), so an unchanged file is
    // genuinely unchanged and the edit is the only delta.
    fx.launch_options = crate::cli::GlobalOptions {
        config: Some(config.clone()),
        ..Default::default()
    };
    let seed_inputs = ReloadInputs::from_launch(&fx.launch_options);
    let old = build_candidate(&seed_inputs).expect("the seed candidate builds");
    fx.app.runtime = Arc::new(old);

    // Unchanged: the System block is the unchanged line.
    fx.app.pending = Some("/reload".to_string());
    drain_through_tick_pending(&mut fx);
    assert!(fx.app.pending.is_none(), "the reload consumed the line");
    let said =
        last_block(&fx.app.transcript, BlockKind::System).expect("the unchanged reload is said");
    assert_eq!(said, "Configuration is unchanged.");

    // Edited: the swap is real (a new value under the same Arc slot the next
    // tick derives its handle from) and the block names the key.
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 7\n",
    )
    .unwrap();
    fx.app.pending = Some("/reload".to_string());
    drain_through_tick_pending(&mut fx);
    assert_eq!(
        fx.app.runtime.resolved.max_rows, 7,
        "the app's runtime handle carries the new value"
    );
    let named =
        last_block(&fx.app.transcript, BlockKind::System).expect("the edited reload is said");
    assert!(
        named.contains("max_rows") && named.contains("100 -> 7"),
        "the plan names the changed key and values: {named}"
    );

    // Broken: an Error block says the previous configuration is still in
    // use, and the handle IS the old one — same allocation, not a fresh
    // Arc over equal contents.
    let before = Arc::clone(&fx.app.runtime);
    std::fs::write(&config, "[run\n").unwrap();
    fx.app.pending = Some("/reload".to_string());
    drain_through_tick_pending(&mut fx);
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the failure is said");
    assert!(
        error.starts_with("Reload failed: ")
            && error.contains("The previous configuration is still in use."),
        "the pinned failure sentence: {error}"
    );
    assert!(
        Arc::ptr_eq(&fx.app.runtime, &before),
        "a failed reload keeps the old runtime handle"
    );
}

/// A `/reload` submitted while a request is running WAITS: the busy gate
/// holds the line in the queue like every command — it never overlaps a
/// stream, query, compaction, or replay — and once idle the queued line
/// drains and the swap happens.
#[test]
fn a_reload_submitted_while_busy_waits_until_idle() {
    use std::sync::Arc;
    let mut fx = ChainFixture::build("reload-busy");
    let config = fx.root.0.join("config.toml");
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 100\n",
    )
    .unwrap();
    fx.launch_options = crate::cli::GlobalOptions {
        config: Some(config.clone()),
        ..Default::default()
    };
    let old = fx.app.runtime.clone();
    let (_tx, rx) = std::sync::mpsc::channel();
    fx.app.compact_task = Some(rx);
    fx.app.pending = Some("/reload".to_string());

    drain_through_tick_pending(&mut fx);
    assert!(
        fx.app.pending.is_some(),
        "while busy, the reload stays queued"
    );
    assert_eq!(
        fx.app.runtime.resolved.max_rows, 100,
        "nothing reloaded while the request runs"
    );

    fx.app.compact_task = None;
    fx.app.pending = Some("/reload".to_string());
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 9\n",
    )
    .unwrap();
    drain_through_tick_pending(&mut fx);
    assert!(fx.app.pending.is_none(), "the queued reload was consumed");
    assert_eq!(
        fx.app.runtime.resolved.max_rows, 9,
        "the swap happened only once idle"
    );
    assert!(
        !Arc::ptr_eq(&fx.app.runtime, &old),
        "the swap is a new handle, never a mutation of the old Arc"
    );
}
