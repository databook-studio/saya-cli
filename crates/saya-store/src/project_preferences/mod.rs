//! The project-preferences repository: explicit, user-confirmed project
//! guidance scoped to one canonical worktree, in its own `project_preferences`
//! table (migration step 9).
//!
//! This is NOT the typed `user_preferences` store — those rows are settings,
//! deliberately prose-free — and NOT typed database knowledge. A preference
//! carries no execution authority: it grants no permission and never changes
//! approvals or sandbox rules. Every bound — 64 per worktree, 1 KiB text,
//! 16 KiB aggregate recall — is enforced inside one transaction, and
//! secret-shaped text is refused, never redacted-and-stored.
//!
//! The migration ladder refuses a database written by a newer binary
//! (`VersionUnsupported`) rather than rewriting it: downgrade = restore from
//! a pre-upgrade backup.

mod reads;
mod writes;

use async_trait::async_trait;
use saya_types::{PreferenceId, ProjectPreference, WorktreeId};

use crate::{SqliteStateStore, StoreError};

/// The project-preferences repository over the `project_preferences` table.
#[async_trait]
pub trait ProjectPreferenceStore: Send + Sync {
    /// Store one explicit preference for `worktree` and return the record:
    /// id minted here, text validated and secret-checked, count and aggregate
    /// bounds enforced inside one transaction; over either bound is refused.
    async fn add_project_preference(
        &self,
        worktree: &WorktreeId,
        text: &str,
        now_unix_ms: i64,
    ) -> Result<ProjectPreference, StoreError>;

    /// Every preference for `worktree`, ordered by creation then id. A row
    /// that fails to decode or disagrees with its own columns is skipped.
    async fn list_project_preferences(
        &self,
        worktree: &WorktreeId,
    ) -> Result<Vec<ProjectPreference>, StoreError>;

    /// Replace one preference's text, keeping its id, worktree, and created
    /// stamp. An unknown or other-worktree id is `NotFound`; a bound the new
    /// text would break is refused with the stored text intact.
    async fn edit_project_preference(
        &self,
        worktree: &WorktreeId,
        id: &PreferenceId,
        new_text: &str,
        now_unix_ms: i64,
    ) -> Result<ProjectPreference, StoreError>;

    /// Forget one preference; `false` for an unknown or other-worktree id.
    async fn forget_project_preference(
        &self,
        worktree: &WorktreeId,
        id: &PreferenceId,
    ) -> Result<bool, StoreError>;

    /// Forget every preference for `worktree`; returns how many rows went.
    async fn forget_all_project_preferences(
        &self,
        worktree: &WorktreeId,
    ) -> Result<usize, StoreError>;
}

#[async_trait]
impl ProjectPreferenceStore for SqliteStateStore {
    async fn add_project_preference(
        &self,
        worktree: &WorktreeId,
        text: &str,
        now_unix_ms: i64,
    ) -> Result<ProjectPreference, StoreError> {
        writes::add_project_preference(self, worktree, text, now_unix_ms).await
    }

    async fn list_project_preferences(
        &self,
        worktree: &WorktreeId,
    ) -> Result<Vec<ProjectPreference>, StoreError> {
        reads::list_project_preferences(self, worktree).await
    }

    async fn edit_project_preference(
        &self,
        worktree: &WorktreeId,
        id: &PreferenceId,
        new_text: &str,
        now_unix_ms: i64,
    ) -> Result<ProjectPreference, StoreError> {
        writes::edit_project_preference(self, worktree, id, new_text, now_unix_ms).await
    }

    async fn forget_project_preference(
        &self,
        worktree: &WorktreeId,
        id: &PreferenceId,
    ) -> Result<bool, StoreError> {
        forget_project_preference(self, worktree, id).await
    }

    async fn forget_all_project_preferences(
        &self,
        worktree: &WorktreeId,
    ) -> Result<usize, StoreError> {
        forget_all_project_preferences(self, worktree).await
    }
}

/// Forget one preference. A single DELETE whose predicate does all the
/// deciding — no bound feeds a write here, so the house `begin()` shape is
/// enough — and `worktree` is the isolation boundary: another worktree's id
/// does not exist here.
pub(crate) async fn forget_project_preference(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
    id: &PreferenceId,
) -> Result<bool, StoreError> {
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    let mut tx = pool.begin().await.map_err(|_| StoreError::Unavailable)?;
    let removed = sqlx::query("DELETE FROM project_preferences WHERE id = ? AND worktree = ?")
        .bind(id.as_str())
        .bind(worktree.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(removed.rows_affected() > 0)
}

/// Forget every preference for `worktree` — and only that worktree's rows.
pub(crate) async fn forget_all_project_preferences(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
) -> Result<usize, StoreError> {
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    let mut tx = pool.begin().await.map_err(|_| StoreError::Unavailable)?;
    let removed = sqlx::query("DELETE FROM project_preferences WHERE worktree = ?")
        .bind(worktree.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(removed.rows_affected() as usize)
}
