//! Read-by-profile and read-by-object for the knowledge-items repository.
//!
//! Object identity is inlined on the row, so profile/object reads need no join.
//! Page queries use deterministic keyset cursors; the compatibility helpers
//! walk those bounded pages when an older caller still asks for a `Vec`.
//! Profile scoping is absolute — the profile id is bound in every query, so a
//! read for one profile can never return another's rows.

use crate::knowledge_items::KnowledgeStoreError;
use crate::knowledge_items::pagination::{
    KnowledgeCursor, KnowledgeItemsQuery, KnowledgeObjectsQuery, KnowledgePage,
};
use crate::knowledge_items::records::{CleanupState, KnowledgeItem};
use crate::{SqliteStateStore, StoreError};
use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity,
};

/// One row's columns, in select order. `value_json` and `schema_binding_json`
/// come back as the serialised strings the store wrote; `value` is decoded
/// through `ClaimPayload`'s serde, the same form it was written under.
type ItemRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
);

const SELECT: &str = "SELECT id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, cleanup_state, fingerprint_version, created_unix_ms, updated_unix_ms FROM knowledge_items";

fn decode_item(row: ItemRow) -> Result<KnowledgeItem, KnowledgeStoreError> {
    let (
        id,
        profile_id,
        catalog,
        schema,
        object,
        object_kind,
        slot,
        cardinality,
        value_json,
        source,
        state,
        schema_binding_json,
        cleanup_state,
        fingerprint_version,
        created_unix_ms,
        updated_unix_ms,
    ) = row;
    let profile = ProfileIdentity::parse(&profile_id).map_err(|_| StoreError::Invalid)?;
    let kind = DatabaseObjectKind::parse(&object_kind).ok_or(StoreError::Invalid)?;
    let object_ref = DatabaseObjectRef::new(profile, &catalog, &schema, &object, kind)
        .map_err(|_| StoreError::Invalid)?;
    // A stored slot that no longer parses is a row written by a build this one
    // cannot read — fail closed as `MalformedSlot` rather than guessing.
    let slot = KnowledgeSlot::parse(&slot).ok_or(KnowledgeStoreError::MalformedSlot)?;
    let value =
        serde_json::from_str::<ClaimPayload>(&value_json).map_err(|_| StoreError::Invalid)?;
    let source = ClaimOrigin::parse(&source).ok_or(StoreError::Invalid)?;
    let state = KnowledgeState::parse(&state).ok_or(StoreError::Invalid)?;
    let cleanup = CleanupState::parse(&cleanup_state).ok_or(StoreError::Invalid)?;
    Ok(KnowledgeItem {
        id,
        object: object_ref,
        slot,
        cardinality_single: cardinality == "single",
        value,
        source,
        state,
        cleanup,
        schema_binding_json,
        fingerprint_version: u32::try_from(fingerprint_version).map_err(|_| StoreError::Invalid)?,
        created_unix_ms,
        updated_unix_ms,
    })
}

/// Every knowledge item for `profile`, collected through bounded pages.
pub(crate) async fn read_for_profile(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError> {
    let mut query = KnowledgeItemsQuery::first_page(super::pagination::MAX_KNOWLEDGE_PAGE_SIZE)
        .map_err(KnowledgeStoreError::from)?;
    let mut items = Vec::new();
    loop {
        let page = read_for_profile_page(store, profile, &query).await?;
        let next = query.next_page(&page);
        items.extend(page.entries);
        let Some(next) = next else { break };
        query = next;
    }
    Ok(items)
}

pub(crate) async fn read_for_profile_page(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
    query: &KnowledgeItemsQuery,
) -> Result<KnowledgePage<KnowledgeItem>, KnowledgeStoreError> {
    let mut sql = format!("{SELECT} WHERE profile_id=?");
    if query.cursor().is_some() {
        sql.push_str(" AND ");
        sql.push_str(profile_cursor_predicate(query.cursor().expect("checked"))?);
    }
    sql.push_str(
        " ORDER BY catalog ASC, schema ASC, object ASC, object_kind ASC, slot ASC, id ASC LIMIT ?",
    );
    let mut statement = sqlx::query_as::<_, ItemRow>(&sql).bind(profile.as_str());
    if let Some(cursor) = query.cursor() {
        let KnowledgeCursor::Profile {
            catalog,
            schema,
            object,
            object_kind,
            slot,
            id,
        } = cursor
        else {
            return Err(StoreError::Invalid.into());
        };
        statement = statement
            .bind(catalog)
            .bind(schema)
            .bind(object)
            .bind(object_kind)
            .bind(slot)
            .bind(id);
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
    let next_cursor = has_more.then(|| KnowledgeCursor::Profile {
        catalog: rows.last().expect("page is non-empty").2.clone(),
        schema: rows.last().expect("page is non-empty").3.clone(),
        object: rows.last().expect("page is non-empty").4.clone(),
        object_kind: rows.last().expect("page is non-empty").5.clone(),
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

/// Every knowledge item for one object, collected through bounded pages.
pub(crate) async fn read_for_object(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError> {
    let mut query = KnowledgeItemsQuery::first_page(super::pagination::MAX_KNOWLEDGE_PAGE_SIZE)
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

/// Retrieve all distinct database objects that have knowledge items for `profile`.
pub(crate) async fn read_objects_for_profile(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
) -> Result<Vec<DatabaseObjectRef>, KnowledgeStoreError> {
    let mut query = KnowledgeObjectsQuery::first_page(super::pagination::MAX_KNOWLEDGE_PAGE_SIZE)
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

fn profile_cursor_predicate(cursor: &KnowledgeCursor) -> Result<&'static str, StoreError> {
    match cursor {
        KnowledgeCursor::Profile { .. } => {
            Ok("(catalog, schema, object, object_kind, slot, id) > (?, ?, ?, ?, ?, ?)")
        }
        _ => Err(StoreError::Invalid),
    }
}

fn object_cursor_predicate(cursor: &KnowledgeCursor) -> Result<&'static str, StoreError> {
    match cursor {
        KnowledgeCursor::Object { .. } => Ok("(slot, id) > (?, ?)"),
        _ => Err(StoreError::Invalid),
    }
}

fn objects_cursor_predicate(cursor: &KnowledgeCursor) -> Result<&'static str, StoreError> {
    match cursor {
        KnowledgeCursor::Objects { .. } => {
            Ok("(catalog, schema, object, object_kind) > (?, ?, ?, ?)")
        }
        _ => Err(StoreError::Invalid),
    }
}
