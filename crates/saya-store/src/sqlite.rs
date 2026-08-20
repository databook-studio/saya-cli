use crate::{StoreError, migration, sqlite_support};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::OnceCell;

/// The longest an opener will spend before reporting the store unavailable.
///
/// Sized for the slowest *legitimate* open, not the fastest. Creating and
/// migrating a new database is quick on a warm local disk and much slower on a
/// cold or contended one — twenty tests creating databases in parallel while a
/// virus scanner reads each new file, or a network share. A six-second budget
/// cancelled those legitimate opens and reported a healthy store as gone.
///
/// A separate `saya` process finishing a WAL checkpoint holds the SQLite write
/// lock briefly; the next opener (the REPL, at startup) used to fail hard on
/// the first busy attempt and report the store as gone. The opener now waits
/// for it within this bound and fails honestly past it — a stuck lock is not a
/// missing store, but an indefinite hang is worse than a typed failure.
pub const OPEN_BUSY_CEILING: Duration = Duration::from_secs(30);

/// Backoff between open attempts while the store is busy. Small enough that a
/// brief sibling checkpoint is noticed promptly, and never a busy-loop: every
/// retry sleeps for at least this long before re-attempting.
const OPEN_BUSY_BACKOFF: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct SqliteStateStore {
    path: Arc<PathBuf>,
    pool: Arc<OnceCell<SqlitePool>>,
}

impl SqliteStateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Arc::new(path.into()),
            pool: Arc::new(OnceCell::new()),
        }
    }

    pub(crate) async fn pool(&self) -> Result<&SqlitePool, StoreError> {
        self.pool
            .get_or_try_init(|| async {
                // A sibling process finishing a WAL checkpoint holds the write
                // lock briefly; the connection's `busy_timeout` covers a single
                // statement window but a lock held longer than that used to fail
                // the open hard on the first attempt. Wait for it within a small
                // bound and retry with backoff; past the ceiling, fail honestly.
                // A database written by a newer saya surfaces as
                // `VersionUnsupported` from `open_once` and is returned at once
                // — that is not busy and is not retried. `Unavailable` (a busy
                // lock, or a malformed file that migration also reports as
                // `Unavailable`) is retried up to the ceiling and then fails
                // honestly; see the report's Gap note for why busy and corrupt
                // are not distinguished here.
                // Two failures are possible here and they need opposite
                // treatment. A *refused* open — a sibling process finishing a
                // WAL checkpoint holds the write lock — should be waited out
                // and retried. A *slow* open — creating and migrating a new
                // database on a cold or contended filesystem — must be left
                // alone to finish.
                //
                // An earlier version budgeted 6s and cancelled the attempt at
                // that point, which killed the second case: on Windows CI,
                // twenty store tests creating databases in parallel with a
                // virus scanner reading each new file, a legitimate open takes
                // longer than six seconds, so it was cancelled, retried,
                // cancelled again and reported as `Unavailable`. Removing the
                // cancellation instead lost the bound altogether: against a
                // held lock, `migrate` waits on sqlx's busy timeout per
                // statement and an open can cost minutes.
                //
                // So the budget is sized for the slowest *legitimate* open, not
                // the fastest. Nothing healthy takes thirty seconds; anything
                // that does is stuck, and failing then is kinder than waiting.
                let deadline = tokio::time::Instant::now() + OPEN_BUSY_CEILING;
                loop {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        return Err(StoreError::Unavailable);
                    }
                    match tokio::time::timeout(remaining, self.open_once()).await {
                        Ok(Ok(pool)) => return Ok(pool),
                        Ok(Err(StoreError::Unavailable)) => {
                            // Refused rather than slow: back off and try again
                            // while budget remains. The sleep keeps this off a
                            // busy-loop; the deadline keeps it bounded.
                            tokio::time::sleep(OPEN_BUSY_BACKOFF).await;
                            continue;
                        }
                        Ok(Err(other)) => return Err(other),
                        // Out of budget mid-attempt: stuck, not slow.
                        Err(_elapsed) => return Err(StoreError::Unavailable),
                    }
                }
            })
            .await
    }

    /// One attempt to open, migrate, and secure the store. Idempotent: every
    /// caller path is safe to run again, so the retry loop above can re-enter it
    /// after a transient busy failure without leaving half-written state.
    async fn open_once(&self) -> Result<SqlitePool, StoreError> {
        sqlite_support::prepare_path(&self.path)?;
        let options = SqliteConnectOptions::new()
            .filename(&*self.path)
            .create_if_missing(true)
            .busy_timeout(Duration::from_secs(5))
            .foreign_keys(true)
            // Overwrite freed content with zeros instead of leaving it
            // in the page. Without this, `forget` blanks a fact's value
            // in the row while the original text stays readable in the
            // database file — the deletion promise honoured in the API
            // and broken in the bytes. `knowledge_security.rs` scans for
            // exactly that.
            .pragma("secure_delete", "ON");
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        migration::migrate(&pool).await?;
        sqlite_support::secure_files(&self.path)?;
        Ok(pool)
    }
    pub async fn close(&self) {
        if let Some(pool) = self.pool.get() {
            pool.close().await;
        }
    }
    pub(crate) fn secure_files(&self) -> Result<(), StoreError> {
        sqlite_support::secure_files(&self.path)
    }
}
