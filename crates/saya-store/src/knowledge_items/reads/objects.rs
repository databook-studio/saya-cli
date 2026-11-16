//! Distinct-object listing for a profile: the database objects that carry
//! knowledge items, through bounded, deterministic keyset pages.

use crate::knowledge_items::KnowledgeStoreError;
use crate::knowledge_items::pagination::{
    KnowledgeCursor, KnowledgeObjectsQuery, KnowledgePage, MAX_KNOWLEDGE_PAGE_SIZE,
};
use crate::{SqliteStateStore, StoreError};
use saya_types::{DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity};

/// Retrieve all distinct database objects that have knowledge items for `profile`.
pub(crate) async fn read_objects_for_profile(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
) -> Result<Vec<DatabaseObjectRef>, KnowledgeStoreError> {
    let mut query = KnowledgeObjectsQuery::first_page(MAX_KNOWLEDGE_PAGE_SIZE)
        .map_err(KnowledgeStoreError::from)?;
    let mut objects = Vec::new();
    loop {
        let page = read_objects_for_profile_page(store, profile, &query).await?;
        let next = query.next_page(&page);
        objects.extend(page.entries);
        let Some(next) = next else { break };
        query = next;
    }
    Ok(objects)
}

pub(crate) async fn read_objects_for_profile_page(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
    query: &KnowledgeObjectsQuery,
) -> Result<KnowledgePage<DatabaseObjectRef>, KnowledgeStoreError> {
    let mut sql = "SELECT DISTINCT catalog, schema, object, object_kind FROM knowledge_items WHERE profile_id=?".to_owned();
    if query.cursor().is_some() {
        sql.push_str(" AND ");
        sql.push_str(objects_cursor_predicate(query.cursor().expect("checked"))?);
    }
    sql.push_str(" ORDER BY catalog ASC, schema ASC, object ASC, object_kind ASC LIMIT ?");
    let mut statement =
        sqlx::query_as::<_, (String, String, String, String)>(&sql).bind(profile.as_str());
    if let Some(cursor) = query.cursor() {
        let KnowledgeCursor::Objects {
            catalog,
            schema,
            object,
            object_kind,
        } = cursor
        else {
            return Err(StoreError::Invalid.into());
        };
        statement = statement
            .bind(catalog)
            .bind(schema)
            .bind(object)
            .bind(object_kind);
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
    let next_cursor = has_more.then(|| {
        let row = rows.last().expect("page is non-empty");
        KnowledgeCursor::Objects {
            catalog: row.0.clone(),
            schema: row.1.clone(),
            object: row.2.clone(),
            object_kind: row.3.clone(),
        }
    });
    let entries = rows
        .into_iter()
        .map(|(catalog, schema, object, object_kind)| {
            let kind = DatabaseObjectKind::parse(&object_kind).ok_or(StoreError::Invalid)?;
            DatabaseObjectRef::new(profile.clone(), &catalog, &schema, &object, kind)
                .map_err(|_| StoreError::Invalid)
        })
        .collect::<Result<_, _>>()?;
    Ok(KnowledgePage {
        entries,
        next_cursor,
    })
}

fn objects_cursor_predicate(cursor: &KnowledgeCursor) -> Result<&'static str, StoreError> {
    match cursor {
        KnowledgeCursor::Objects { .. } => {
            Ok("(catalog, schema, object, object_kind) > (?, ?, ?, ?)")
        }
        _ => Err(StoreError::Invalid),
    }
}
