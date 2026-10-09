//! The forget path: erase an item's content in the same transaction that
//! marks it `Dismissed`, keep the row as a tombstone, then complete physical
//! cleanup (or leave it pending for a retry).

use crate::contracts::now;
use crate::knowledge_items::KnowledgeStoreError;
use crate::knowledge_items::records::CleanupState;
use crate::{SqliteStateStore, StoreError};
use saya_types::KnowledgeState;

use super::cleanup::cleanup_files;

/// The serialised binding a forgotten row is reduced to: `SchemaBinding::Table`,
/// which names no column. The deletion promise (`docs/memory.md` §Deletion) is
/// that a forgotten fact's referenced columns are cleared, and a binding that
/// depends only on the table existing carries no column dependency. It is valid
/// `SchemaBinding` JSON, so a read that decodes a dismissed row's binding does
/// not fail — though no live path reads a dismissed row's binding (validity
/// returns `Invalid` for `Dismissed` before consulting it, and `confirm`
/// refuses a dismissed row with `Conflict` before revalidation).
const BLANKED_BINDING_JSON: &str = r#"{"type":"table"}"#;

/// Forget `id`: erase its content in the same transaction that marks it
/// `Dismissed`, then keep the row as a tombstone.
///
/// `value_json` is replaced with the serialised [`ClaimPayload::blanked`] of the
/// row's current value — the same variant with its free-text fields emptied, so
/// the secret-bearing channels (a description, a grain, an alias where a user or
/// the model could have pasted a credential) carry nothing. `schema_binding_json`
/// is replaced with [`BLANKED_BINDING_JSON`], which names no column. The `id`,
/// object identity, `slot`, `source`, `fingerprint_version`, and timestamps are
/// untouched, so "why did SAYA stop using that?" stays answerable and a
/// re-remember of the same fact lands on the same row (its id is value-independent
/// for a single-valued slot, and the row keeps its id for a multi-valued one) to
/// report *previously forgotten* rather than silently resurrecting.
///
/// One transaction: a crash cannot leave a `Dismissed` row that still holds its
/// content. The row is read inside the tx so the blanked value is derived from
/// what is actually stored, not from a caller-supplied echo; an unknown id is
/// `NotFound`. The blanked value is re-serialised through `ClaimPayload`'s serde
/// (the same form it was written under), so a later read decodes it — clearing
/// `value_json` to a non-`ClaimPayload` JSON like `null` would break every read
/// of the object, because `decode_item` deserialises the value of every row
/// including dismissed ones.
pub(crate) async fn forget_item(
    store: &SqliteStateStore,
    id: &str,
) -> Result<crate::knowledge_items::ForgetOutcome, KnowledgeStoreError> {
    let stamp = now();
    let mut tx = store
        .pool()
        .await
        .map_err(|_| StoreError::Unavailable)?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT value_json, state, cleanup_state FROM knowledge_items WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    let Some((value_json, state, cleanup_state)) = row else {
        // An unknown id is a typed `NotFound`, not a silent no-op — the caller
        // asked to forget a fact that is not there.
        tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
        return Err(StoreError::NotFound.into());
    };
    if state == KnowledgeState::Dismissed.as_str()
        && cleanup_state == CleanupState::Pending.as_str()
    {
        tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
        return finish_cleanup(store, id).await;
    }
    // Derive the blanked payload from the stored value rather than trusting a
    // caller's claim about it. A row written by an incompatible build whose
    // value this build cannot decode cannot be safely blanked — fail closed
    // rather than write a `Dismissed` row whose value is not what we read.
    let payload: saya_types::ClaimPayload =
        serde_json::from_str(&value_json).map_err(|_| StoreError::Invalid)?;
    let blanked = serde_json::to_string(&payload.blanked()).map_err(|_| StoreError::Invalid)?;
    sqlx::query(
        "UPDATE knowledge_items SET value_json=?, schema_binding_json=?, state='dismissed', cleanup_state='pending', updated_unix_ms=? WHERE id=?",
    )
    .bind(&blanked)
    .bind(BLANKED_BINDING_JSON)
    .bind(stamp)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    finish_cleanup(store, id).await
}

/// Complete physical cleanup after the logical tombstone commits. If the
/// checkpoint fails, the row remains pending and a later call can retry it.
async fn finish_cleanup(
    store: &SqliteStateStore,
    id: &str,
) -> Result<crate::knowledge_items::ForgetOutcome, KnowledgeStoreError> {
    if let Err(error) = cleanup_files(store).await {
        return match error {
            KnowledgeStoreError::Store(_) => {
                Ok(crate::knowledge_items::ForgetOutcome::CleanupPending)
            }
            other => Err(other),
        };
    }
    let marked = sqlx::query(
        "UPDATE knowledge_items SET cleanup_state='complete', updated_unix_ms=? WHERE id=?",
    )
    .bind(now())
    .bind(id)
    .execute(store.pool().await.map_err(|_| StoreError::Unavailable)?)
    .await;
    if marked.is_err() {
        return Ok(crate::knowledge_items::ForgetOutcome::CleanupPending);
    }
    Ok(crate::knowledge_items::ForgetOutcome::Cleaned)
}
