use super::connection_schema_reconcile::reconcile_after_refresh;
use super::{
    connection, connection_schema_cache,
    output::{emit, failure},
    state,
};
use crate::{
    config::runtime::RuntimeConfig,
    profile_identity::profile_identity,
    render::{RenderFormat, TerminalEvent},
};
use saya_connectors::DatabaseConnector;
use saya_store::{AuditOperation, AuditStatus, SchemaStore, SqliteStateStore};
use std::time::Instant;

pub(crate) async fn run(
    name: &str,
    refresh: bool,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    store: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let profile = match runtime.named_profile(name) {
        Ok(profile) => profile,
        Err(error) => {
            return failure(
                3,
                saya_types::ConnectionError::invalid_configuration(error.to_string()),
                format,
            );
        }
    };
    // The identity is derived once as a `ProfileIdentity` so the reconciliation
    // pass (which keys claims to schema by profile) gets the same value the
    // store keys schema by, without re-parsing the string form.
    let identity = profile_identity(name, profile, &runtime.cache_scope);
    let identity_str = identity.as_str();
    let mut persistence_failed = refresh && store.invalidate_schema(identity_str).await.is_err();
    let started = Instant::now();
    let connector = match connection::build(profile, runtime, can_prompt).await {
        Ok(connector) => connector,
        Err(error) if !refresh => {
            return connection_schema_cache::fallback(
                store,
                identity_str,
                started,
                error,
                format,
                &mut persistence_failed,
            )
            .await;
        }
        Err(error) => {
            persistence_failed |= state::audit_silent(
                store,
                identity_str,
                AuditOperation::SchemaRefresh,
                AuditStatus::Failure,
                started.elapsed(),
                None,
                None,
            )
            .await
            .is_err();
            warn(persistence_failed, format);
            return failure(3, error, format);
        }
    };
    match live_schema(&*connector).await {
        Ok(schema) => {
            persistence_failed |= store.upsert_schema(identity_str, &schema).await.is_err();
            persistence_failed |= state::audit_silent(
                store,
                identity_str,
                AuditOperation::SchemaRefresh,
                AuditStatus::Success,
                started.elapsed(),
                None,
                None,
            )
            .await
            .is_err();
            warn(persistence_failed, format);
            emit(
                TerminalEvent::Schema {
                    schema: schema.clone(),
                },
                format,
            );
            // An explicit refresh is the one path that reconciles: it has a
            // fresh live schema in hand, and a user triggered it, so the cost
            // is expected and the state change is not silent. A plain view, a
            // per-turn auto-load, and a cache fallback never reach here.
            if refresh {
                reconcile_after_refresh(store, &identity, &schema, format).await;
            }
            Ok(0)
        }
        Err(error) if !refresh => {
            connection_schema_cache::fallback(
                store,
                identity_str,
                started,
                error,
                format,
                &mut persistence_failed,
            )
            .await
        }
        Err(error) => {
            persistence_failed |= state::audit_silent(
                store,
                identity_str,
                AuditOperation::SchemaRefresh,
                AuditStatus::Failure,
                started.elapsed(),
                None,
                None,
            )
            .await
            .is_err();
            warn(persistence_failed, format);
            failure(3, error, format)
        }
    }
}

async fn live_schema(
    connector: &dyn DatabaseConnector,
) -> Result<saya_types::SchemaTree, saya_types::ConnectionError> {
    connector.connect().await?;
    connector.schema().await
}

fn warn(persistence_failed: bool, format: RenderFormat) {
    if persistence_failed {
        state::diagnostic(format);
    }
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::{
        capture_output_start, capture_output_take, cli::GlobalOptions,
        config::runtime::load_with_sources, profile_identity::profile_identity,
        render::RenderFormat,
    };
    use saya_store::{ContractStore, ProposeClaim, ProposeOutcome, SchemaStore, SqliteStateStore};
    use saya_types::{
        ClaimOrigin, ClaimPayload, ClaimStatus, Column, DatabaseObjectKind, DatabaseObjectRef,
        SchemaFingerprint, SchemaTree, Table,
    };
    use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
    use std::{collections::BTreeMap, fs, path::Path, time::SystemTime};

    /// A process-unique temp root.
    fn temp_root(label: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("saya-schema-reconcile-{label}-{stamp}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// A one-profile sqlite runtime whose `data.sqlite3` is the live database.
    /// The sqlite connector reports the file stem (`data`) as the catalog and
    /// `main` as the schema, so the claim's object must use those names.
    fn runtime_at(root: &Path) -> (crate::config::runtime::RuntimeConfig, std::path::PathBuf) {
        let database = root.join("data.sqlite3");
        fs::write(&database, b"").unwrap();
        let connections = root.join("connections.toml");
        fs::write(
            &connections,
            format!(
                "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
                database.display()
            ),
        )
        .unwrap();
        let options = GlobalOptions {
            connections: Some(connections.clone()),
            ..Default::default()
        };
        let runtime = load_with_sources(&options, root, root, BTreeMap::new()).unwrap();
        (runtime, database)
    }

    fn orders_table(with_amount: bool) -> Table {
        let mut cols = vec![Column {
            name: "id".into(),
            data_type: "INTEGER".into(),
            nullable: false,
        }];
        if with_amount {
            cols.push(Column {
                name: "amount".into(),
                data_type: "INTEGER".into(),
                nullable: false,
            });
        }
        Table {
            name: "orders".into(),
            columns: cols,
        }
    }

    /// A store with migrations run.
    async fn store_at(root: &Path) -> SqliteStateStore {
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let (runtime, _) = runtime_at(root);
        let identity = profile_identity(
            "local",
            runtime.named_profile("local").unwrap(),
            &runtime.cache_scope,
        );
        store
            .upsert_schema(identity.as_str(), &SchemaTree::default())
            .await
            .unwrap();
        store
    }

    /// Creates `data.sqlite3` with `orders(id, amount)` and inserts a confirmed
    /// contract claim referencing `amount`, then drops `amount` from the live
    /// database and runs an explicit schema refresh. The refresh must reconcile
    /// against the new live schema, mark the claim stale, and report it.
    #[tokio::test]
    async fn refresh_reconciles_a_drifted_claim_and_reports_it() {
        let root = temp_root("wiring");
        let (runtime, database) = runtime_at(&root);
        let store = store_at(&root).await;

        // 1. Create the live database with orders(id, amount).
        let options = SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::query("CREATE TABLE orders (id INTEGER NOT NULL, amount INTEGER NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        // 2. Propose a confirmed claim on orders(id, amount) referencing `amount`.
        let profile = runtime.named_profile("local").unwrap();
        let identity = profile_identity("local", profile, &runtime.cache_scope);
        let base = orders_table(true);
        let payload = ClaimPayload::column_description("amount", "how much").unwrap();
        let request = ProposeClaim {
            object: DatabaseObjectRef::new(
                identity.clone(),
                "data",
                "main",
                "orders",
                DatabaseObjectKind::Table,
            )
            .unwrap(),
            fingerprint: SchemaFingerprint::of_table(DatabaseObjectKind::Table, &base),
            referenced_columns: payload.referenced_column_snapshots(&base),
            payload,
            origin: ClaimOrigin::UserExplicit,
            initial_status: ClaimStatus::Confirmed,
            evidence: None,
        };
        let claim_id = match store.propose_claim(request).await.unwrap() {
            ProposeOutcome::Stored(id) => id,
            other => panic!("expected Stored, got {other:?}"),
        };

        // 3. Drop `amount` from the live database.
        let pool = SqlitePool::connect_with(SqliteConnectOptions::new().filename(&database))
            .await
            .unwrap();
        sqlx::query("ALTER TABLE orders DROP COLUMN amount")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        // 4. Explicit refresh: fetches orders(id), reconciles, reports.
        capture_output_start();
        let code = run("local", true, &runtime, RenderFormat::Text, false, &store)
            .await
            .unwrap();
        let (stdout, stderr) = capture_output_take();
        assert_eq!(code, 0, "refresh should succeed: {stdout}{stderr}");
        // The schema event is on stdout; the reconcile diagnostic on stderr.
        assert!(stdout.contains("orders"), "schema event missing: {stdout}");
        assert!(
            stderr.contains("marked stale"),
            "reconcile diagnostic missing: {stderr}"
        );
        assert!(
            stderr.contains("1"),
            "diagnostic should report one claim examined/marked: {stderr}"
        );

        // 5. The claim is now persisted Stale.
        let after = store.get_claim(&claim_id).await.unwrap().unwrap();
        assert_eq!(after.status, ClaimStatus::Stale);

        let _ = fs::remove_dir_all(root);
    }

    /// A refresh with no live schema for the profile must NOT mark: reconcile
    /// is skipped on the cache-fallback / connect-failure paths. Here the
    /// connector cannot reach the database (the file is removed before the
    /// refresh connects), so the refresh fails and no claim is marked.
    #[tokio::test]
    async fn a_failed_refresh_does_not_mark_claims() {
        let root = temp_root("wiring_no_schema");
        let (runtime, database) = runtime_at(&root);
        let store = store_at(&root).await;

        // Create orders(id, amount) and a confirmed claim referencing amount.
        let options = SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::query("CREATE TABLE orders (id INTEGER NOT NULL, amount INTEGER NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let profile = runtime.named_profile("local").unwrap();
        let identity = profile_identity("local", profile, &runtime.cache_scope);
        let base = orders_table(true);
        let payload = ClaimPayload::column_description("amount", "how much").unwrap();
        let request = ProposeClaim {
            object: DatabaseObjectRef::new(
                identity.clone(),
                "data",
                "main",
                "orders",
                DatabaseObjectKind::Table,
            )
            .unwrap(),
            fingerprint: SchemaFingerprint::of_table(DatabaseObjectKind::Table, &base),
            referenced_columns: payload.referenced_column_snapshots(&base),
            payload,
            origin: ClaimOrigin::UserExplicit,
            initial_status: ClaimStatus::Confirmed,
            evidence: None,
        };
        let claim_id = match store.propose_claim(request).await.unwrap() {
            ProposeOutcome::Stored(id) => id,
            other => panic!("expected Stored, got {other:?}"),
        };

        // Remove the database file so the refresh cannot connect (refresh=true
        // does not fall back to cache — it reports failure). Reconcile must not
        // run, so the claim stays Confirmed.
        fs::remove_file(&database).unwrap();
        capture_output_start();
        let _ = run("local", true, &runtime, RenderFormat::Text, false, &store).await;
        let (stdout, stderr) = capture_output_take();
        assert!(
            !stderr.contains("marked stale"),
            "a failed refresh must not reconcile: {stdout}{stderr}"
        );
        let after = store.get_claim(&claim_id).await.unwrap().unwrap();
        assert_eq!(
            after.status,
            ClaimStatus::Confirmed,
            "a failed refresh must not mark"
        );

        let _ = fs::remove_dir_all(root);
    }
}
