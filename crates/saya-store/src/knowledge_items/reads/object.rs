//! Object-scoped knowledge-item reads: every item of one object through
//! bounded, deterministic keyset pages.

use super::row::{ItemRow, SELECT, decode_item};
use crate::knowledge_items::KnowledgeStoreError;
use crate::knowledge_items::pagination::{
    KnowledgeCursor, KnowledgeItemsQuery, KnowledgePage, MAX_KNOWLEDGE_PAGE_SIZE,
};
use crate::knowledge_items::records::KnowledgeItem;
use crate::{SqliteStateStore, StoreError};
use saya_types::DatabaseObjectRef;

/// Every knowledge item for one object, collected through bounded pages.
pub(crate) async fn read_for_object(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError> {
    let mut query = KnowledgeItemsQuery::first_page(MAX_KNOWLEDGE_PAGE_SIZE)
        .map_err(KnowledgeStoreError::from)?;
    let mut items = Vec::new();
    loop {
        let page = read_for_object_page(store, object, &query).await?;
        let next = query.next_page(&page);
        items.extend(page.entries);
        let Some(next) = next else { break };
        query = next;
    }
    Ok(items)
}

pub(crate) async fn read_for_object_page(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    query: &KnowledgeItemsQuery,
) -> Result<KnowledgePage<KnowledgeItem>, KnowledgeStoreError> {
    let mut sql = format!(
        "{SELECT} WHERE profile_id=? AND catalog=? AND schema=? AND object=? AND object_kind=?"
    );
    if query.cursor().is_some() {
        sql.push_str(" AND ");
        sql.push_str(object_cursor_predicate(query.cursor().expect("checked"))?);
    }
    sql.push_str(" ORDER BY slot ASC, id ASC LIMIT ?");
    let mut statement = sqlx::query_as::<_, ItemRow>(&sql)
        .bind(object.profile().as_str())
        .bind(object.catalog())
        .bind(object.schema())
        .bind(object.object())
        .bind(object.kind().as_str());
    if let Some(cursor) = query.cursor() {
        let KnowledgeCursor::Object { slot, id } = cursor else {
            return Err(StoreError::Invalid.into());
        };
        statement = statement.bind(slot).bind(id);
    }
    let mut rows = statement
        .bind(i64::try_from(query.limit().saturating_add(1)).map_err(|_| StoreError::Invalid)?)
        .fetch_all(store.pool().await.map_err(|_| StoreError::Unavailable)?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let has_more = rows.len() > query.limit();
    if has_more {
        rows.truncate(query.limit());
    }
    let next_cursor = has_more.then(|| KnowledgeCursor::Object {
        slot: rows.last().expect("page is non-empty").6.clone(),
        id: rows.last().expect("page is non-empty").0.clone(),
    });
    Ok(KnowledgePage {
        entries: rows
            .into_iter()
            .map(decode_item)
            .collect::<Result<_, _>>()?,
        next_cursor,
    })
}

fn object_cursor_predicate(cursor: &KnowledgeCursor) -> Result<&'static str, StoreError> {
    match cursor {
        KnowledgeCursor::Object { .. } => Ok("(slot, id) > (?, ?)"),
        _ => Err(StoreError::Invalid),
    }
}
