//! E2-3: a re-run entry (`--refresh`, `/chart`, bare `/explain`) must
//! refuse when the stored alias now resolves to a different source — the old
//! SQL must not silently run against the wrong database. Snapshot and report
//! read the capture only and keep working, printing the old provenance.

use super::{Dispatch, dispatch};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::application::tests_support::unused_runtime;
use crate::interactive::tui::capture::CapturedResult;
use crate::interactive::tui::transcript::{BlockKind, Transcript};
use crate::interactive::tui::types::LastQuery;
use crate::render::RenderFormat;
use saya_agent::ApprovalPolicy;
use saya_store::FsSessionStore;
use saya_types::{
    DatabaseProfile, EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult,
    SqlDialect,
};
use std::path::{Path, PathBuf};

const STARTED_UNIX_MS: i64 = 1_700_000_000_000;

/// Local dispatch fixture: the exact pieces the real `dispatch` takes,
/// backed by a throwaway root. (`chain_tests::ChainFixture` is private to
/// its module, so this mirrors its shape rather than importing it.)
struct GuardFixture {
    state: SessionState,
    session: SessionRuntime,
    store: FsSessionStore,
    state_db: saya_store::SqliteStateStore,
    runtime: RuntimeConfig,
    transcript: Transcript,
    last_query: Option<LastQuery>,
    captured: Option<CapturedResult>,
    root: PathBuf,
}

impl GuardFixture {
    fn build(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-source-guard-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("guard test root");
        let mut runtime = unused_runtime();
        runtime
            .connections
            .profiles
            .insert("demo".to_string(), sqlite(&root.join("demo.sqlite3")));
        let session = SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            "guard-test",
            ApprovalPolicy::Ask,
            &root.join("sessions"),
        )
        .expect("the guard test session acquires");
        Self {
            state: SessionState::new("guard-test", None, "model"),
            session,
            store: FsSessionStore::new(root.join("session-store")),
            state_db: saya_store::SqliteStateStore::new(root.join("state.sqlite3")),
            runtime,
            transcript: Transcript::new(),
            last_query: None,
            captured: None,
            root,
        }
    }

    /// The state a completed `/sql` leaves behind: the last query plus its
    /// captured result and evidence, in memory only.
    fn with_capture(&mut self, identity: Option<String>) {
        self.last_query = Some(LastQuery {
            sql: "SELECT 1".to_string(),
            connection: Some("demo".to_string()),
        });
        let result = sample_result();
        let evidence = ExecutionEvidence::for_result(
            &result,
            ExecutionEvidenceArgs {
                execution_id: "xabc-1".to_string(),
                connection_label: "demo".to_string(),
                connection_identity: identity,
                dialect: SqlDialect::Sqlite,
                max_rows: 100,
                started_unix_ms: STARTED_UNIX_MS,
                finished_unix_ms: STARTED_UNIX_MS + 2_000,
                source: EvidenceSource::DirectSql,
            },
        );
        self.captured = Some(CapturedResult { result, evidence });
    }

    /// Rebinds the `demo` alias to a DIFFERENT sqlite path — same alias,
    /// different resolved source.
    fn rebind_demo(&mut self) {
        self.runtime
            .connections
            .profiles
            .insert("demo".to_string(), sqlite(&self.root.join("other.sqlite3")));
    }

    fn dispatch(&mut self, line: &str) -> Dispatch {
        let profiles: Vec<String> = Vec::new();
        dispatch(
            line,
            &mut self.transcript,
            &profiles,
            &mut self.state,
            &self.runtime,
            &self.store,
            &self.state_db,
            RenderFormat::Text,
            &mut self.last_query,
            &self.captured,
            None,
            &mut self.session,
        )
    }

    fn error_text(&self) -> Option<String> {
        self.transcript
            .blocks()
            .iter()
            .rev()
            .find(|block| block.kind == BlockKind::Error)
            .map(|block| block.text.clone())
    }
}

impl Drop for GuardFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn sqlite(path: &Path) -> DatabaseProfile {
    DatabaseProfile::Sqlite {
        path: path.display().to_string(),
        read_only: true,
    }
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

fn current_identity(runtime: &RuntimeConfig) -> String {
    let profile = runtime.named_profile("demo").expect("demo resolves");
    crate::profile_identity("demo", profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}

/// Control: with nothing rebound, `--refresh` re-runs normally.
#[test]
fn refresh_reruns_when_source_unchanged() {
    let mut fx = GuardFixture::build("control");
    fx.with_capture(Some(current_identity(&fx.runtime)));
    let path = fx.root.join("refresh.csv");
    let outcome = fx.dispatch(&format!("/export --refresh {}", path.display()));
    assert!(
        matches!(outcome, Dispatch::SqlTask(_)),
        "an unchanged source re-runs the last query"
    );
    assert!(
        fx.error_text()
            .is_none_or(|text| !text.contains("Refused to re-run")),
        "no refusal is said on the control path"
    );
}

/// MISMATCH: after the alias is rebound, each re-run entry is refused and
/// spawns no SQL task.
#[test]
fn rerun_entries_refuse_after_alias_rebind() {
    for tag in ["refresh", "chart", "explain"] {
        let mut fx = GuardFixture::build(tag);
        fx.with_capture(Some(current_identity(&fx.runtime)));
        fx.rebind_demo();
        let line = match tag {
            "refresh" => format!(
                "/export --refresh {}",
                fx.root.join("refresh.csv").display()
            ),
            "chart" => format!("/chart line {}", fx.root.join("chart.png").display()),
            _ => "/explain".to_string(),
        };
        let outcome = fx.dispatch(&line);
        assert!(
            matches!(outcome, Dispatch::Handled),
            "{tag}: a mismatched re-run is handled inline, spawning nothing"
        );
        let text = fx.error_text().expect("{tag}: the refusal is said");
        assert!(
            text.contains("Refused to re-run"),
            "{tag}: the refusal names itself: {text}"
        );
        assert!(
            text.contains("different source"),
            "{tag}: the refusal names the cause: {text}"
        );
        assert!(
            !text.contains("sentinel-alice"),
            "{tag}: no row content leaks"
        );
    }
}

/// Snapshot and report never re-run: after the rebind they still read the
/// OLD capture, labelled with its original provenance.
#[test]
fn snapshot_and_report_keep_reading_the_old_capture() {
    let mut fx = GuardFixture::build("snapshot-report");
    fx.with_capture(Some(current_identity(&fx.runtime)));
    let short_id = fx
        .captured
        .as_ref()
        .expect("capture")
        .evidence
        .short_id()
        .to_owned();
    fx.rebind_demo();

    let snap = fx.root.join("snapshot.csv");
    let outcome = fx.dispatch(&format!("/export --snapshot {}", snap.display()));
    assert!(
        matches!(outcome, Dispatch::Handled),
        "snapshot stays inline"
    );
    let written = std::fs::read_to_string(&snap).expect("snapshot written");
    assert!(
        written.contains("sentinel-alice"),
        "rows come from the capture"
    );
    let said = fx
        .transcript
        .blocks()
        .iter()
        .rev()
        .find(|block| block.kind == BlockKind::System)
        .map(|block| block.text.clone())
        .expect("snapshot success is said");
    assert!(
        said.contains(&short_id),
        "the old execution is named: {said}"
    );

    let report = fx.root.join("report.md");
    let outcome = fx.dispatch(&format!("/report --rows 2 {}", report.display()));
    assert!(matches!(outcome, Dispatch::Handled), "report stays inline");
    let written = std::fs::read_to_string(&report).expect("report written");
    assert!(
        written.contains("sentinel-alice"),
        "rows come from the capture"
    );
    assert!(
        written.contains("Connection label: demo"),
        "old provenance kept"
    );
}

/// Unknown stays unknown: evidence without an identity re-runs as before.
#[test]
fn unknown_identity_reruns_as_before() {
    let mut fx = GuardFixture::build("unknown");
    fx.with_capture(None);
    fx.rebind_demo();
    let path = fx.root.join("refresh.csv");
    let outcome = fx.dispatch(&format!("/export --refresh {}", path.display()));
    assert!(
        matches!(outcome, Dispatch::SqlTask(_)),
        "without a stored identity there is nothing to compare, so the re-run proceeds"
    );
}

/// The pure guard's unknown cases: no capture, no identity, a different
/// label, or an unresolvable label all mean "no proven mismatch".
#[test]
fn pure_guard_unknown_stays_unknown() {
    let fx = GuardFixture::build("pure");
    let stored = current_identity(&fx.runtime);
    let evidence = |label: &str, identity: Option<String>| {
        ExecutionEvidence::for_result(
            &sample_result(),
            ExecutionEvidenceArgs {
                execution_id: "xabc-1".to_string(),
                connection_label: label.to_string(),
                connection_identity: identity,
                dialect: SqlDialect::Sqlite,
                max_rows: 100,
                started_unix_ms: STARTED_UNIX_MS,
                finished_unix_ms: STARTED_UNIX_MS + 2_000,
                source: EvidenceSource::DirectSql,
            },
        )
    };
    assert!(
        super::query_source_guard::rerun_source_mismatch(None, "demo", &fx.runtime).is_none(),
        "no capture, no refusal"
    );
    assert!(
        super::query_source_guard::rerun_source_mismatch(
            Some(&evidence("demo", None)),
            "demo",
            &fx.runtime
        )
        .is_none(),
        "no stored identity, no refusal"
    );
    assert!(
        super::query_source_guard::rerun_source_mismatch(
            Some(&evidence("demo", Some(stored.clone()))),
            "other",
            &fx.runtime
        )
        .is_none(),
        "a different label is not this re-run's business"
    );
    assert!(
        super::query_source_guard::rerun_source_mismatch(
            Some(&evidence("ghost", Some(stored))),
            "ghost",
            &fx.runtime
        )
        .is_none(),
        "an unresolvable label keeps the existing error path"
    );
}
