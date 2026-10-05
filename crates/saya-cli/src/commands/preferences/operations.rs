//! The shared project-preference operations (M04a): one bounded CRUD family
//! over explicit, user-confirmed per-worktree guidance, returning typed
//! outcomes the CLI subcommand renders and the slash packet (M04b) reuses.
//!
//! Every operation is worktree-scoped by the caller's `WorktreeId`, needs no
//! database profile, and creates nothing the user did not type: there is no
//! model-facing constructor here and no capability rides a record. Store
//! errors map onto [`PreferenceRefusal`] so every surface refuses with the
//! same words.

use super::{PreferenceOutcome, PreferenceRefusal};
use crate::interactive::session_continuation::now_unix_ms;
use saya_store::{ProjectPreferenceStore, SqliteStateStore, StoreError};
use saya_types::{MAX_PROJECT_PREFERENCES, PreferenceId, WorktreeId};

/// Store one explicit preference for `worktree`.
pub(crate) async fn add_preference(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
    text: &str,
) -> PreferenceOutcome {
    match store
        .add_project_preference(worktree, text, now_unix_ms())
        .await
    {
        Ok(record) => PreferenceOutcome::Added(record),
        Err(error) => PreferenceOutcome::Refused(refusal_for(error, store, worktree).await),
    }
}

/// Every preference for `worktree`, ordered by creation then id.
pub(crate) async fn list_preferences(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
) -> PreferenceOutcome {
    match store.list_project_preferences(worktree).await {
        Ok(items) => PreferenceOutcome::Listed {
            worktree_label: worktree.as_str().to_owned(),
            items,
        },
        Err(error) => PreferenceOutcome::Refused(refusal_for(error, store, worktree).await),
    }
}

/// Replace one preference's text. A malformed id is refused as
/// [`PreferenceRefusal::InvalidId`] before any store call, so an
/// unvalidated id never reaches the store.
pub(crate) async fn edit_preference(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
    id: &str,
    text: &str,
) -> PreferenceOutcome {
    let Ok(id) = PreferenceId::parse(id) else {
        return PreferenceOutcome::Refused(PreferenceRefusal::InvalidId);
    };
    match store
        .edit_project_preference(worktree, &id, text, now_unix_ms())
        .await
    {
        Ok(record) => PreferenceOutcome::Edited(record),
        Err(error) => PreferenceOutcome::Refused(refusal_for(error, store, worktree).await),
    }
}

/// Forget one preference; the id shape is checked before any store call.
pub(crate) async fn forget_preference(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
    id: &str,
) -> PreferenceOutcome {
    let Ok(id) = PreferenceId::parse(id) else {
        return PreferenceOutcome::Refused(PreferenceRefusal::InvalidId);
    };
    match store.forget_project_preference(worktree, &id).await {
        Ok(existed) => PreferenceOutcome::Forgotten {
            existed,
            id: id.as_str().to_owned(),
        },
        Err(error) => PreferenceOutcome::Refused(refusal_for(error, store, worktree).await),
    }
}

/// Forget every preference for `worktree`; reports how many rows went.
pub(crate) async fn forget_all_preferences(
    store: &SqliteStateStore,
    worktree: &WorktreeId,
) -> PreferenceOutcome {
    match store.forget_all_project_preferences(worktree).await {
        Ok(removed) => PreferenceOutcome::ForgottenAll(removed),
        Err(error) => PreferenceOutcome::Refused(refusal_for(error, store, worktree).await),
    }
}

/// The (exit-class) mapping of a store error onto a refusal. `LimitExceeded`
/// covers two bounds, so the current list distinguishes them: a full count
/// is `TooMany`, anything else is the aggregate `TooMuchText`. If that
/// secondary read fails, the aggregate is the safer wording — the count cap
/// involves no text, so it cannot be the reason a *new text* was refused.
async fn refusal_for(
    error: StoreError,
    store: &SqliteStateStore,
    worktree: &WorktreeId,
) -> PreferenceRefusal {
    match error {
        StoreError::Invalid => PreferenceRefusal::TextNotAccepted,
        StoreError::LimitExceeded => {
            let count = store
                .list_project_preferences(worktree)
                .await
                .map(|items| items.len())
                .unwrap_or(MAX_PROJECT_PREFERENCES.saturating_sub(1));
            if count >= MAX_PROJECT_PREFERENCES {
                PreferenceRefusal::TooMany
            } else {
                PreferenceRefusal::TooMuchText
            }
        }
        StoreError::NotFound => PreferenceRefusal::NotFound,
        // `StoreError` is `#[non_exhaustive]`: anything unclassifiable is a
        // store-level failure, surfaced as unavailable, never silently.
        _ => PreferenceRefusal::Unavailable,
    }
}
