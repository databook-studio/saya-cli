//! Single-item lookup by unique ID.

use super::row::{ItemRow, SELECT, decode_item};
use crate::knowledge_items::KnowledgeStoreError;
use crate::knowledge_items::records::KnowledgeItem;
use crate::{SqliteStateStore, StoreError};

/// Retrieve a single knowledge item by its unique ID.
pub(crate) async fn read_by_id(
    store: &SqliteStateStore,
    id: &str,
) -> Result<Option<KnowledgeItem>, KnowledgeStoreError> {
    let sql = format!("{SELECT} WHERE id=?");
    let row = sqlx::query_as::<_, ItemRow>(&sql)
        .bind(id)
        .fetch_optional(store.pool().await.map_err(|_| StoreError::Unavailable)?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    row.map(decode_item).transpose()
}
