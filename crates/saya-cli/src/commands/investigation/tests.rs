//! Unit tests for the investigation command operations (S6). The dispatcher
//! is driven with an explicit repository (`run_investigation_in`) so no test
//! touches process env; the binary-level surface is covered in
//! `tests/investigation_cli.rs`.

use super::{RunOutcome, run_investigation_in, run_investigation_outcome_in};
use crate::cli::InvestigationCommand;
use crate::commands::{capture_output_start, capture_output_take};
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::profile_identity::profile_identity;
use crate::render::RenderFormat;
use saya_store::{InvestigationRepository, SqliteStateStore};
use saya_types::investigation::InvestigationDefinitionV1;
use saya_types::{EvidenceSource, ResultScope, SqlDialect};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

// -- harness ---------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-cli-unit-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A single-profile runtime over a sqlite database file; the one-profile
/// connections file auto-selects `local` as the active profile, so save
/// resolves it without `--connection`.
fn runtime_at(root: &Path, connections_toml: &str) -> RuntimeConfig {
    let connections = root.join("connections.toml");
    fs::write(&connections, connections_toml).unwrap();
    let options = crate::cli::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
}

fn single_profile_toml(root: &Path) -> String {
    let database = root.join("data.sqlite3");
    fs::write(&database, b"").unwrap();
    format!(
        "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
        database.display()
    )
}

async fn run(
    repo: &InvestigationRepository,
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
) -> (i32, String, String) {
    capture_output_start();
    // The dispatcher threads a state store for the async `run` arm only;
    // these sync-arm tests never touch it, so a scratch path suffices.
    let state_db =
        SqliteStateStore::new(std::env::temp_dir().join("saya-investigation-unit-state"));
    let code = run_investigation_in(repo, command, runtime, RenderFormat::Text, false, &state_db)
        .await
        .unwrap();
    let (out, err) = capture_output_take();
    (code, out, err)
}

fn repo_at(root: &Path) -> InvestigationRepository {
    InvestigationRepository::new(root.join("investigations"))
}

fn document_path(root: &Path, id: &str) -> PathBuf {
    root.join("investigations").join(format!("{id}.json"))
}

fn binding_path(root: &Path, id: &str) -> PathBuf {
    root.join("investigations")
        .join("local")
        .join(format!("{id}.json"))
}

/// Saves one investigation and returns its id, for tests that start from a
/// saved state.
async fn saved_id(repo: &InvestigationRepository, runtime: &RuntimeConfig) -> String {
    let (code, out, err) = run(
        repo,
        InvestigationCommand::Save {
            name: "Order events".into(),
            description: Some("events in order".into()),
            sql: Some("SELECT id, label FROM events ORDER BY id".into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        runtime,
    )
    .await;
    assert_eq!(code, 0, "save failed: {out}{err}");
    out.lines()
        .next()
        .expect("save prints the id first")
        .to_string()
}

fn expected_identity(runtime: &RuntimeConfig, name: &str) -> String {
    let profile = runtime.named_profile(name).unwrap();
    profile_identity(name, profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}

// -- save ------------------------------------------------------------------

#[tokio::test]
async fn save_roundtrip_exact_sql_binding_list_and_show() {
    let root = temp_root("roundtrip");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    // Exact spacing survives: the document keeps the SQL verbatim.
    let sql = "SELECT   id ,  label FROM events";
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Order events".into(),
            description: Some("events in order".into()),
            sql: Some(sql.into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    assert!(out.contains(
        "Saved exactly as shown. Review the SQL before sharing: literals are stored verbatim."
    ));
    let id = out.lines().next().unwrap().to_string();
    assert!(id.starts_with("order-events-"), "id line: {id}");

    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    let definition = InvestigationDefinitionV1::from_json_bytes(document.as_bytes()).unwrap();
    assert_eq!(definition.sql, sql, "the SQL must be stored exactly");
    assert_eq!(definition.objects, vec!["events".to_string()]);
    assert_eq!(definition.connection, "local");
    assert_eq!(definition.dialect, SqlDialect::Sqlite);
    assert_eq!(definition.revision, 1);
    assert_eq!(definition.name, "Order events");
    assert!(
        binding_path(&root, &id).exists(),
        "binding beside the document"
    );
    let binding_text = fs::read_to_string(binding_path(&root, &id)).unwrap();
    assert!(binding_text.contains("\"profile_identity\""));

    let (code, out, err) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "err: {err}");
    assert!(
        out.contains(&format!("{id}  1  sqlite  local  Order events")),
        "list line: {out}"
    );

    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Show { id: id.clone() },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "err: {err}");
    assert!(out.contains(sql), "show prints the exact SQL: {out}");
    assert!(
        out.contains("local binding: local (reviewed revision 1)"),
        "{out}"
    );
    let identity = expected_identity(&runtime, "local");
    assert!(
        !out.contains(&identity),
        "show must never print the identity: {out}"
    );
    assert!(
        !document.contains(&identity),
        "the document must never carry the identity"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn save_refuses_write_sql_with_exit_4_and_writes_nothing() {
    let root = temp_root("write-sql");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Wipe".into(),
            description: None,
            sql: Some("DELETE FROM events".into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 4, "out: {out} err: {err}");
    assert!(err.contains("read-only safety policy"), "err: {err}");
    assert!(
        !root.join("investigations").join("local").exists(),
        "nothing written"
    );
    assert!(repo.list(0, 1).unwrap().summaries.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn save_refuses_credential_shaped_sql() {
    let root = temp_root("credential");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Leak".into(),
            description: None,
            sql: Some("SELECT 'password=hunter2'".into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2, "out: {out} err: {err}");
    assert!(err.contains("credential-shaped text"), "err: {err}");
    assert!(
        repo.list(0, 1).unwrap().summaries.is_empty(),
        "nothing written"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn save_requires_a_connection_when_none_resolves() {
    let root = temp_root("no-connection");
    // An empty connections file resolves no profile at all.
    let runtime = runtime_at(&root, "");
    let repo = repo_at(&root);
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Lonely".into(),
            description: None,
            sql: Some("SELECT 1".into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2, "out: {out} err: {err}");
    assert!(
        err.contains("no connection: pass --connection <profile>"),
        "err: {err}"
    );
    assert!(repo.list(0, 1).unwrap().summaries.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn save_refuses_sql_and_file_together() {
    let root = temp_root("both");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Both".into(),
            description: None,
            sql: Some("SELECT 1".into()),
            file: Some(root.join("unused.sql")),
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2, "out: {out} err: {err}");
    assert!(err.contains("not both"), "err: {err}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn save_refuses_invalid_names_and_oversize_sql_with_typed_messages() {
    let root = temp_root("bounds");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    async fn save_with(
        repo: &InvestigationRepository,
        runtime: &RuntimeConfig,
        name: &str,
        sql: String,
    ) -> (i32, String, String) {
        run(
            repo,
            InvestigationCommand::Save {
                name: name.into(),
                description: None,
                sql: Some(sql),
                file: None,
                connection: None,
                param_specs: Vec::new(),
            },
            runtime,
        )
        .await
    }
    let (code, _, err) = save_with(&repo, &runtime, "   ", "SELECT 1".into()).await;
    assert_eq!(code, 2, "an empty name is a usage error");
    assert!(err.contains("name must be 1-80 characters"), "err: {err}");
    let over = format!(
        "SELECT '{}'",
        "x".repeat(saya_types::investigation::MAX_SQL_BYTES)
    );
    let (code, _, err) = save_with(&repo, &runtime, "Big", over).await;
    assert_eq!(code, 2);
    assert!(err.contains("at most 65536 bytes"), "err: {err}");
    let _ = fs::remove_dir_all(root);
}

// -- list ------------------------------------------------------------------

#[tokio::test]
async fn list_empty_says_no_saved_investigations() {
    let root = temp_root("list-empty");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "err: {err}");
    assert_eq!(out, "No saved investigations.\n");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn list_reports_unreadable_documents_as_warnings() {
    let root = temp_root("list-warning");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    fs::create_dir_all(root.join("investigations")).unwrap();
    fs::write(
        root.join("investigations").join("bad-doc-1.json"),
        "not json at all",
    )
    .unwrap();
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "a bad file warns, never fails the page: {err}");
    assert!(out.contains("warning: bad-doc-1"), "out: {out}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn list_clamps_limit_and_honours_offset() {
    let root = temp_root("list-page");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    for (first, second) in [("First", "SELECT 1"), ("Second", "SELECT 2")] {
        let (code, out, err) = run(
            &repo,
            InvestigationCommand::Save {
                name: first.into(),
                description: None,
                sql: Some(second.into()),
                file: None,
                connection: None,
                param_specs: Vec::new(),
            },
            &runtime,
        )
        .await;
        assert_eq!(code, 0, "out: {out} err: {err}");
    }
    // limit 0 and limit 500 both clamp into the repository's 1..=50 page.
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::List {
            limit: Some(0),
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "limit 0 clamps, never errors: {err}");
    assert!(
        out.contains("more: --offset 1"),
        "ordinary pagination names the next offset: {out}"
    );
    let summaries = out
        .lines()
        .filter(|line| !line.starts_with("more:"))
        .count();
    assert_eq!(summaries, 1, "limit 0 clamps to one summary: {out}");
    let (_, out, _) = run(
        &repo,
        InvestigationCommand::List {
            limit: Some(500),
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(out.lines().count(), 2, "both fit inside the 50-page: {out}");
    assert!(
        !out.contains("(capped at 500)") && !out.contains("more:"),
        "nothing beyond this page: {out}"
    );
    // offset skips past the first id in id order.
    let (_, all, _) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
    )
    .await;
    let (_, shifted, _) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: Some(1),
        },
        &runtime,
    )
    .await;
    let first_of = |text: &str| text.lines().next().unwrap().to_string();
    assert_eq!(shifted.trim(), all.lines().nth(1).unwrap().trim());
    assert_ne!(first_of(&shifted), first_of(&all));
    let _ = fs::remove_dir_all(root);
}

// -- show ------------------------------------------------------------------

#[tokio::test]
async fn show_refuses_missing_malformed_newer_and_corrupt_ids() {
    let root = temp_root("show-refusals");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let id = saved_id(&repo, &runtime).await;

    // Malformed id: same refusal shape as a missing one.
    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Show {
            id: "NOT AN ID".into(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2);
    assert!(err.contains("no investigation"), "err: {err}");

    // A document written by a newer saya is refused, never downgraded.
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    fs::write(
        document_path(&root, &id),
        document.replace("\"version\": 1", "\"version\": 2"),
    )
    .unwrap();
    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Show { id: id.clone() },
        &runtime,
    )
    .await;
    assert_eq!(code, 2);
    assert!(err.contains("made by a newer saya"), "err: {err}");

    // A corrupt document is reported, never repaired or silently shown.
    fs::write(document_path(&root, &id), "not json at all").unwrap();
    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Show { id: id.clone() },
        &runtime,
    )
    .await;
    assert_eq!(code, 2);
    assert!(err.contains("corrupt or invalid"), "err: {err}");

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn show_reports_a_missing_binding_honestly() {
    let root = temp_root("show-binding");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let id = saved_id(&repo, &runtime).await;
    fs::remove_file(binding_path(&root, &id)).unwrap();
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Show { id: id.clone() },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "err: {err}");
    assert!(out.contains("local binding: none"), "out: {out}");
    let _ = fs::remove_dir_all(root);
}

// -- delete ----------------------------------------------------------------

#[tokio::test]
async fn delete_checks_revision_then_removes_document_and_binding() {
    let root = temp_root("delete");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let id = saved_id(&repo, &runtime).await;

    // A stale --revision is refused with the current revision named.
    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Delete {
            id: id.clone(),
            revision: Some(5),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2, "err: {err}");
    assert!(err.contains("at revision 1"), "err: {err}");
    assert!(
        document_path(&root, &id).exists(),
        "refused delete changes nothing"
    );

    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Delete {
            id: id.clone(),
            revision: Some(1),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    assert!(!document_path(&root, &id).exists(), "document removed");
    assert!(!binding_path(&root, &id).exists(), "binding removed too");

    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Show { id: id.clone() },
        &runtime,
    )
    .await;
    assert_eq!(code, 2);
    assert!(
        err.contains(&format!("no investigation {id}")),
        "err: {err}"
    );
    let (_, out, _) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(out, "No saved investigations.\n");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn delete_refuses_a_document_it_cannot_verify() {
    let root = temp_root("delete-corrupt");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let id = saved_id(&repo, &runtime).await;
    fs::write(document_path(&root, &id), "not json at all").unwrap();
    let (code, _, err) = run(
        &repo,
        InvestigationCommand::Delete {
            id: id.clone(),
            revision: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2, "a corrupt document is never deleted: {err}");
    assert!(err.contains("corrupt or invalid"), "err: {err}");
    assert!(document_path(&root, &id).exists(), "the file stays");
    let _ = fs::remove_dir_all(root);
}

// -- run outcome (C3/D12) --------------------------------------------------

/// Seeds the events table straight into the profile's sqlite file, so a
/// replay has something bounded to read.
async fn seed_events(database: &Path) {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO events (id, label) VALUES (1, 'first'), (2, 'second')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// Drives the dispatcher's typed-outcome seam with output captured, the way
/// the TUI replay adapter consumes a run (D12): the outcome, not just the
/// exit code.
async fn run_outcome(
    repo: &InvestigationRepository,
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    state_db: &SqliteStateStore,
) -> (RunOutcome, String, String) {
    capture_output_start();
    let outcome =
        run_investigation_outcome_in(repo, command, runtime, RenderFormat::Text, false, state_db)
            .await
            .unwrap();
    let (out, err) = capture_output_take();
    (outcome, out, err)
}

fn run_command(id: &str) -> InvestigationCommand {
    InvestigationCommand::Run {
        id: id.to_string(),
        connection: None,
        revalidate: false,
        report: None,
        rows: None,
        overwrite: false,
        params: Vec::new(),
    }
}

#[tokio::test]
async fn run_outcome_carries_result_and_evidence_on_success() {
    let root = temp_root("outcome-success");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime).await;
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));

    let (outcome, out, err) = run_outcome(&repo, run_command(&id), &runtime, &state_db).await;
    assert_eq!(outcome.code, 0, "out: {out} err: {err}");
    let replay = outcome
        .replay
        .expect("a successful replay carries its result and evidence");
    assert_eq!(replay.sql, "SELECT id, label FROM events ORDER BY id");
    assert_eq!(replay.connection, "local");
    assert_eq!(replay.result.columns, ["id", "label"]);
    assert_eq!(replay.result.row_count, 2);
    assert_eq!(replay.evidence.returned_rows, 2);
    assert_eq!(replay.evidence.connection_label, "local");
    assert_eq!(replay.evidence.dialect, SqlDialect::Sqlite);
    assert!(matches!(replay.evidence.scope, ResultScope::Full));
    assert_eq!(
        replay.evidence.source,
        EvidenceSource::SavedInvestigation {
            id: id.clone(),
            revision: 1,
        }
    );
    // The same events are still emitted in the same order (invariant 2):
    // the rendered evidence line names the saved source and the profile.
    assert!(out.contains("saved investigation"), "out: {out}");
    assert!(out.contains("2 rows"), "out: {out}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn run_outcome_has_no_replay_on_stale_or_failure() {
    let root = temp_root("outcome-none");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime).await;
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));

    // Stale: the document's revision moved on disk under the binding, so the
    // run is refused before anything executes and no replay exists.
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    fs::write(
        document_path(&root, &id),
        document.replace("\"revision\": 1", "\"revision\": 2"),
    )
    .unwrap();
    let (stale, out, err) = run_outcome(&repo, run_command(&id), &runtime, &state_db).await;
    assert_eq!(stale.code, 2, "out: {out} err: {err}");
    assert!(stale.replay.is_none(), "a stale refusal carries no replay");
    assert!(err.contains("revision changed"), "err: {err}");

    // Execution failure: the SQL passed save's read-only shape gate but the
    // column does not exist, so the connector refuses it — the safety exit
    // carries no replay either.
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Broken".into(),
            description: None,
            sql: Some("SELECT nosuchcol FROM events".into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "save passed the shape gate: {out} err: {err}");
    let broken = out.lines().next().unwrap().to_string();
    let (failed, out, err) = run_outcome(&repo, run_command(&broken), &runtime, &state_db).await;
    assert_eq!(failed.code, 4, "out: {out} err: {err}");
    assert!(
        failed.replay.is_none(),
        "a failed execution carries no replay"
    );
    assert!(err.contains("nosuchcol"), "err: {err}");

    let _ = fs::remove_dir_all(root);
}
