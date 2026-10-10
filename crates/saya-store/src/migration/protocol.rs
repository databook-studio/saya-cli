use super::{foundation, knowledge, operational};
use crate::{StoreError, coding, sqlite_support};
use sqlx::{Sqlite, SqlitePool, pool::PoolConnection};
use std::time::Duration;

const LOCK_RETRIES: usize = 100;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);

pub(crate) async fn migrate(pool: &SqlitePool) -> Result<(), StoreError> {
    let mut connection = pool.acquire().await.map_err(|_| StoreError::Unavailable)?;
    retry_statement(&mut connection, "BEGIN IMMEDIATE").await?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let did_work = match version {
        0 => {
            foundation::step1(&mut connection).await?;
            foundation::step2(&mut connection).await?;
            foundation::step3(&mut connection).await?;
            knowledge::step4(&mut connection).await?;
            knowledge::step5(&mut connection).await?;
            operational::step6(&mut connection).await?;
            operational::step7(&mut connection).await?;
            knowledge::step8(&mut connection).await?;
            operational::step9(&mut connection).await?;
            coding::schema::step10(&mut connection).await?;
            coding::schema::step11(&mut connection).await?;
            true
        }
        1 => {
            foundation::step2(&mut connection).await?;
            foundation::step3(&mut connection).await?;
            knowledge::step4(&mut connection).await?;
            knowledge::step5(&mut connection).await?;
            operational::step6(&mut connection).await?;
            operational::step7(&mut connection).await?;
            knowledge::step8(&mut connection).await?;
            operational::step9(&mut connection).await?;
            coding::schema::step10(&mut connection).await?;
            coding::schema::step11(&mut connection).await?;
            true
        }
        2 => {
            foundation::step3(&mut connection).await?;
            knowledge::step4(&mut connection).await?;
            knowledge::step5(&mut connection).await?;
            operational::step6(&mut connection).await?;
            operational::step7(&mut connection).await?;
            knowledge::step8(&mut connection).await?;
            operational::step9(&mut connection).await?;
            coding::schema::step10(&mut connection).await?;
            coding::schema::step11(&mut connection).await?;
            true
        }
        3 => {
            knowledge::step4(&mut connection).await?;
            knowledge::step5(&mut connection).await?;
            operational::step6(&mut connection).await?;
            operational::step7(&mut connection).await?;
            knowledge::step8(&mut connection).await?;
            operational::step9(&mut connection).await?;
            coding::schema::step10(&mut connection).await?;
            coding::schema::step11(&mut connection).await?;
            true
        }
        4 => {
            knowledge::step5(&mut connection).await?;
            operational::step6(&mut connection).await?;
            operational::step7(&mut connection).await?;
            knowledge::step8(&mut connection).await?;
            operational::step9(&mut connection).await?;
            coding::schema::step10(&mut connection).await?;
            coding::schema::step11(&mut connection).await?;
            true
        }
        5 => {
            operational::step6(&mut connection).await?;
            operational::step7(&mut connection).await?;
            knowledge::step8(&mut connection).await?;
            operational::step9(&mut connection).await?;
            coding::schema::step10(&mut connection).await?;
            coding::schema::step11(&mut connection).await?;
            true
        }
        6 => {
            operational::step7(&mut connection).await?;
            knowledge::step8(&mut connection).await?;
            operational::step9(&mut connection).await?;
            coding::schema::step10(&mut connection).await?;
            coding::schema::step11(&mut connection).await?;
            true
        }
        7 => {
            knowledge::step8(&mut connection).await?;
            operational::step9(&mut connection).await?;
            coding::schema::step10(&mut connection).await?;
            coding::schema::step11(&mut connection).await?;
            true
        }
        8 => {
            operational::step9(&mut connection).await?;
            coding::schema::step10(&mut connection).await?;
            coding::schema::step11(&mut connection).await?;
            true
        }
        9 => {
            coding::schema::step10(&mut connection).await?;
            coding::schema::step11(&mut connection).await?;
            true
        }
        10 => {
            coding::schema::step11(&mut connection).await?;
            true
        }
        11 => false,
        _ => {
            sqlx::query("ROLLBACK").execute(&mut *connection).await.ok();
            return Err(StoreError::VersionUnsupported);
        }
    };
    sqlx::query("COMMIT")
        .execute(&mut *connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    if did_work {
        retry_statement(&mut connection, "PRAGMA journal_mode = WAL").await?;
    }
    Ok(())
}

async fn retry_statement(
    connection: &mut PoolConnection<Sqlite>,
    statement: &str,
) -> Result<(), StoreError> {
    for _ in 0..LOCK_RETRIES {
        match sqlx::query(statement).execute(&mut **connection).await {
            Ok(_) => return Ok(()),
            Err(error) => {
                let mapped = sqlite_support::map_open_error(&error);
                if mapped != StoreError::Unavailable {
                    return Err(mapped);
                }
            }
        }
        tokio::time::sleep(LOCK_RETRY_DELAY).await;
    }
    Err(StoreError::Unavailable)
}
