//! Unit tests for `saya investigation edit` (A2.5 reopened): an edit is a
//! new revision through the shared dispatcher (`run_investigation_in`), the
//! same save-time gates guard the replacement SQL, the id, dialect, and
//! connection alias never change, and the local review binding is left
//! stale by revision — the next run refuses until `--revalidate`. The
//! harness mirrors `tests.rs` but is self-contained: those helpers are
//! private to that module.

use crate::cli::InvestigationCommand;
use crate::commands::investigation::run_investigation_in;
use crate::commands::{capture_output_start, capture_output_take};
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::render::RenderFormat;
use saya_store::{InvestigationRepository, SqliteStateStore};
use saya_types::SqlDialect;
use saya_types::investigation::InvestigationDefinitionV1;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

// -- harness ---------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-edit-unit-{label}-{}",
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
    state_db: &SqliteStateStore,
) -> (i32, String, String) {
    capture_output_start();
    let code = run_investigation_in(repo, command, runtime, RenderFormat::Text, false, state_db)
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

async fn saved_id(
    repo: &InvestigationRepository,
    runtime: &RuntimeConfig,
    state: &SqliteStateStore,
) -> String {
    let (code, out, err) = run(
        repo,
        InvestigationCommand::Save {
            name: "Order events".into(),
            description: Some("events in order".into()),
            sql: Some("SELECT id, label FROM events ORDER BY id".into()),
            file: None,
            connection: None,
        },
        runtime,
        state,
    )
    .await;
    assert_eq!(code, 0, "save failed: {out}{err}");
    out.lines()
        .next()
        .expect("save prints the id first")
        .to_string()
}

fn edit_command(
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
    sql: Option<&str>,
    file: Option<PathBuf>,
) -> InvestigationCommand {
    InvestigationCommand::Edit {
        id: id.to_string(),
        name: name.map(Into::into),
        description: description.map(Into::into),
        sql: sql.map(Into::into),
        file,
    }
}

fn run_command(id: &str, revalidate: bool) -> InvestigationCommand {
    InvestigationCommand::Run {
        id: id.to_string(),
        connection: None,
        revalidate,
        report: None,
        rows: None,
        overwrite: false,
    }
}

/// Seeds the `events` table straight into the profile's sqlite file, so a
/// replay after `--revalidate` has something bounded to read.
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

// -- edit ------------------------------------------------------------------

#[tokio::test]
async fn edit_sql_bumps_revision_and_the_next_run_refuses_until_revalidate() {
    let root = temp_root("arc");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime, &state).await;

    let (code, out, err) = run(
        &repo,
        edit_command(
            &id,
            None,
            None,
            Some("SELECT label FROM events WHERE id = 2"),
            None,
        ),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    assert!(out.contains("\"revision\": 2"), "the new JSON: {out}");
    assert!(
        out.contains("Edited to revision 2. The next run needs --revalidate."),
        "out: {out}"
    );
    assert!(
        out.contains("SELECT label FROM events WHERE id = 2"),
        "the new SQL is echoed: {out}"
    );

    // The document on disk: revision 2, new SQL, recomputed objects, and
    // the immutable id, dialect, connection, and creation time.
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    let definition = InvestigationDefinitionV1::from_json_bytes(document.as_bytes()).unwrap();
    assert_eq!(definition.revision, 2);
    assert_eq!(definition.sql, "SELECT label FROM events WHERE id = 2");
    assert_eq!(definition.objects, vec!["events".to_string()]);
    assert_eq!(definition.id.as_str(), id);
    assert_eq!(definition.dialect, SqlDialect::Sqlite);
    assert_eq!(definition.connection, "local");
    assert_eq!(definition.name, "Order events");
    assert_eq!(definition.description.as_deref(), Some("events in order"));

    // The binding is not rewritten: it still reviews revision 1.
    let binding = fs::read_to_string(binding_path(&root, &id)).unwrap();
    assert!(
        binding.contains("\"reviewed_revision\": 1"),
        "binding: {binding}"
    );

    // The next run refuses as stale; --revalidate accepts and replays.
    let (code, _, err) = run(&repo, run_command(&id, false), &runtime, &state).await;
    assert_eq!(code, 2, "err: {err}");
    assert!(err.contains("revision changed"), "err: {err}");
    assert!(err.contains("--revalidate"), "err: {err}");
    let (code, out, err) = run(&repo, run_command(&id, true), &runtime, &state).await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    assert!(out.contains("saved investigation"), "out: {out}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_keeps_untouched_fields_and_replaces_the_given_ones() {
    let root = temp_root("name-description");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;

    // A name edit keeps the description.
    let (code, out, err) = run(
        &repo,
        edit_command(&id, Some("Renamed events"), None, None, None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    let definition = InvestigationDefinitionV1::from_json_bytes(document.as_bytes()).unwrap();
    assert_eq!(definition.name, "Renamed events");
    assert_eq!(definition.description.as_deref(), Some("events in order"));
    assert_eq!(definition.sql, "SELECT id, label FROM events ORDER BY id");
    assert_eq!(definition.revision, 2);

    // A description edit keeps the name; the SQL is re-gated even unchanged.
    let (code, out, err) = run(
        &repo,
        edit_command(&id, None, Some("fresh words"), None, None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    assert!(
        out.contains("Edited to revision 3. The next run needs --revalidate."),
        "out: {out}"
    );
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    let definition = InvestigationDefinitionV1::from_json_bytes(document.as_bytes()).unwrap();
    assert_eq!(definition.name, "Renamed events");
    assert_eq!(definition.description.as_deref(), Some("fresh words"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_reads_the_replacement_sql_from_file() {
    let root = temp_root("file");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let replacement = root.join("replacement.sql");
    fs::write(&replacement, "SELECT count(*) FROM events").unwrap();
    let (code, _, err) = run(
        &repo,
        edit_command(&id, None, None, None, Some(replacement)),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 0, "err: {err}");
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    let definition = InvestigationDefinitionV1::from_json_bytes(document.as_bytes()).unwrap();
    assert_eq!(definition.sql, "SELECT count(*) FROM events");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_refuses_write_sql_with_exit_4_and_changes_nothing() {
    let root = temp_root("write-sql");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let document_before = fs::read(document_path(&root, &id)).unwrap();
    let binding_before = fs::read(binding_path(&root, &id)).unwrap();
    let (code, _, err) = run(
        &repo,
        edit_command(&id, None, None, Some("DELETE FROM events"), None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 4, "err: {err}");
    assert!(err.contains("read-only safety policy"), "err: {err}");
    assert_eq!(
        fs::read(document_path(&root, &id)).unwrap(),
        document_before,
        "a refused edit changes nothing"
    );
    assert_eq!(fs::read(binding_path(&root, &id)).unwrap(), binding_before);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_refuses_credential_shaped_sql() {
    let root = temp_root("credential");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let (code, _, err) = run(
        &repo,
        edit_command(&id, None, None, Some("SELECT 'password=hunter2'"), None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 2, "err: {err}");
    assert!(err.contains("credential-shaped text"), "err: {err}");
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    assert!(document.contains("\"revision\": 1"), "nothing written");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_with_no_fields_is_a_usage_error() {
    let root = temp_root("no-fields");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let (code, _, err) = run(
        &repo,
        edit_command(&id, None, None, None, None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 2, "err: {err}");
    assert!(
        err.contains("edit needs a change") && err.contains("--sql") && err.contains("--name"),
        "err: {err}"
    );
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    assert!(document.contains("\"revision\": 1"), "nothing written");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_refuses_sql_and_file_together() {
    let root = temp_root("both");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let (code, _, err) = run(
        &repo,
        edit_command(
            &id,
            None,
            None,
            Some("SELECT 1"),
            Some(root.join("unused.sql")),
        ),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 2, "err: {err}");
    assert!(err.contains("not both"), "err: {err}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_surfaces_a_stale_expected_revision_as_a_conflict() {
    let root = temp_root("conflict");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    // Another editor wins the race between this edit's read and its
    // optimistic update. The store refuses a revision it cannot advance;
    // a document parked at the maximum revision makes that refusal
    // deterministic (`expected.checked_add(1)` overflows) without racing
    // two processes.
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    fs::write(
        document_path(&root, &id),
        document.replace("\"revision\": 1", "\"revision\": 4294967295"),
    )
    .unwrap();
    let binding_before = fs::read(binding_path(&root, &id)).unwrap();
    let (code, _, err) = run(
        &repo,
        edit_command(&id, Some("Renamed"), None, None, None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 2, "err: {err}");
    assert!(
        err.contains("changed underneath this command; retry"),
        "err: {err}"
    );
    // Nothing changed: the document keeps the concurrent editor's revision
    // and name, and the binding is untouched.
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    assert!(document.contains("\"revision\": 4294967295"), "{document}");
    assert!(document.contains("Order events"), "{document}");
    assert_eq!(fs::read(binding_path(&root, &id)).unwrap(), binding_before);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_refuses_an_invalid_name_with_save_s_message() {
    let root = temp_root("bad-name");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let (code, _, err) = run(
        &repo,
        edit_command(&id, Some("   "), None, None, None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 2, "an empty name is a usage error: {err}");
    assert!(err.contains("name must be 1-80 characters"), "err: {err}");
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    assert!(document.contains("\"revision\": 1"), "nothing written");
    let _ = fs::remove_dir_all(root);
}
