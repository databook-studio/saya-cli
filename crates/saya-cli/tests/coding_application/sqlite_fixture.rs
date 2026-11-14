//! Selected SQLite setup shared by database-capability journey tests.

use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{CodingApplication, GlobalOptions, load_with_sources};

pub(crate) struct SelectedSqlite {
    pub(crate) pool: sqlx::SqlitePool,
    pub(crate) elsewhere: std::path::PathBuf,
}

pub(crate) async fn select_scratch_sqlite(fixture: &mut Fixture, sharing: bool) -> SelectedSqlite {
    let database = fixture.root.join("data.sqlite3");
    let elsewhere = fixture.root.join("must-not-connect.sqlite3");
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TABLE saya_sentinel (value TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO saya_sentinel VALUES ('approved-row'), ('second-row')")
        .execute(&pool)
        .await
        .unwrap();
    let missing = fixture.root.join("missing.sqlite");
    let connections = format!(
        "[profiles.zzmissing]\ntype='sqlite'\npath='{}'\nread_only=false\n[profiles.scratch]\ntype='sqlite'\npath='{}'\n[profiles.elsewhere]\ntype='sqlite'\npath='{}'\n",
        missing.display(),
        database.display(),
        elsewhere.display()
    );
    let external = fixture.root.join("connections-under-test.toml");
    std::fs::write(&external, connections).unwrap();
    std::fs::create_dir(fixture.repository.join(".saya")).unwrap();
    std::os::unix::fs::symlink(&external, fixture.repository.join(".saya/connections.toml"))
        .unwrap();
    let depth = std::env::current_dir().unwrap().ancestors().count() - 1;
    let relative = std::path::PathBuf::from("../".repeat(depth))
        .join(fixture.repository.strip_prefix("/").unwrap());
    let loaded = load_with_sources(
        &GlobalOptions {
            profile: Some("scratch".into()),
            ..Default::default()
        },
        &relative,
        &fixture.root.join("user-config"),
        Default::default(),
    )
    .unwrap();
    fixture.runtime.connections = loaded.connections;
    fixture.runtime.connections_path = loaded.connections_path;
    fixture.runtime.cache_scope = loaded.cache_scope;
    fixture.runtime.resolved.ai.allow_data_sharing = sharing;
    fixture.runtime.resolved.max_rows = 1;
    fixture.application = CodingApplication::open(
        fixture.runtime.clone(),
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    SelectedSqlite { pool, elsewhere }
}
