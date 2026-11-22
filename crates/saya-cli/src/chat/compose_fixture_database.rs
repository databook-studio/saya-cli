//! The one seeded database a composition test's session resolves.
//!
//! Split from [`super::compose_fixture`] so that file keeps its own line cap:
//! this is the SQLite half — a table with two rows, and the connection entry
//! carrying the profile identity a stored claim is filed under.

use std::path::Path;

use saya_connectors::{ConnectorOptions, SqliteConnector};
use saya_types::{DatabaseProfile, SqlDialect};

use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// One resolved ceiling as the finite limit the configuration carries: no
/// fixture states zero, and zero is not a limit the parser accepts.
pub(crate) fn limit(value: u64) -> saya_config::BudgetLimit {
    saya_config::BudgetLimit::Finite(
        std::num::NonZeroU64::new(value).expect("a stated fixture ceiling is nonzero"),
    )
}

/// One seeded SQLite connection, named `primary` — the name a chat grants its
/// "yes for this run" under.
pub(crate) async fn sqlite_connection(root: &Path) -> ConnectionRegistry {
    let path = root.join("main.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .expect("the fixture database creates");
    sqlx::query("CREATE TABLE IF NOT EXISTS users (id INTEGER, name TEXT)")
        .execute(&seed)
        .await
        .expect("the fixture table creates");
    sqlx::query("INSERT INTO users VALUES (1, 'ada'), (2, 'grace')")
        .execute(&seed)
        .await
        .expect("the fixture rows insert");
    seed.close().await;
    let connector = SqliteConnector::open(&path, true, ConnectorOptions::default())
        .await
        .expect("the fixture database opens");
    // The entry carries the profile identity a stored claim is filed under, so
    // recall can resolve the profile it selects against — the same identity the
    // connection's own profile would hash to.
    let identity = crate::profile_identity::profile_identity(
        "primary",
        &DatabaseProfile::Sqlite {
            path: path.display().to_string(),
            read_only: true,
        },
        &root.join("connections.toml"),
    );
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(connector),
            dialect: SqlDialect::Sqlite,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    registry
}
