use super::super::{Dispatch, dispatch};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
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

pub(super) const STARTED_UNIX_MS: i64 = 1_700_000_000_000;

/// Local dispatch fixture: the exact pieces the real `dispatch` takes,
/// backed by a throwaway root. (`chain_tests::ChainFixture` is private to
/// its module, so this mirrors its shape rather than importing it.)
pub(super) struct GuardFixture {
    state: SessionState,
    session: SessionRuntime,
    store: FsSessionStore,
    state_db: saya_store::SqliteStateStore,
    pub(super) runtime: RuntimeConfig,
    pub(super) transcript: Transcript,
    last_query: Option<LastQuery>,
    pub(super) captured: Option<CapturedResult>,
    pub(super) root: PathBuf,
}

impl GuardFixture {
    pub(super) fn build(tag: &str) -> Self {
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
    pub(super) fn with_capture(&mut self, identity: Option<String>) {
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
    pub(super) fn rebind_demo(&mut self) {
        self.runtime
            .connections
            .profiles
            .insert("demo".to_string(), sqlite(&self.root.join("other.sqlite3")));
    }

    pub(super) fn dispatch(&mut self, line: &str) -> Dispatch {
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

    pub(super) fn error_text(&self) -> Option<String> {
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

pub(super) fn sample_result() -> QueryResult {
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

pub(super) fn current_identity(runtime: &RuntimeConfig) -> String {
    let profile = runtime.named_profile("demo").expect("demo resolves");
    crate::profile_identity("demo", profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}
