//! Bounded keyset queries for knowledge items and their object index.

use crate::StoreError;

/// The largest page a knowledge query may return. Callers use continuation
/// cursors to walk further rows; no single query can materialise an archive.
pub const MAX_KNOWLEDGE_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeCursor {
    Profile {
        catalog: String,
        schema: String,
        object: String,
        object_kind: String,
        slot: String,
        id: String,
    },
    Object {
        slot: String,
        id: String,
    },
    Objects {
        catalog: String,
        schema: String,
        object: String,
        object_kind: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgePage<T> {
    pub entries: Vec<T>,
    pub next_cursor: Option<KnowledgeCursor>,
}

impl<T> KnowledgePage<T> {
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeItemsQuery {
    limit: usize,
    cursor: Option<KnowledgeCursor>,
}

impl KnowledgeItemsQuery {
    pub fn first_page(limit: usize) -> Result<Self, StoreError> {
        validate_limit(limit).map(|limit| Self {
            limit,
            cursor: None,
        })
    }

    pub fn next_page<T>(&self, page: &KnowledgePage<T>) -> Option<Self> {
        page.next_cursor.clone().map(|cursor| Self {
            limit: self.limit,
            cursor: Some(cursor),
        })
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn cursor(&self) -> Option<&KnowledgeCursor> {
        self.cursor.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeObjectsQuery {
    limit: usize,
    cursor: Option<KnowledgeCursor>,
}

impl KnowledgeObjectsQuery {
    pub fn first_page(limit: usize) -> Result<Self, StoreError> {
        validate_limit(limit).map(|limit| Self {
            limit,
            cursor: None,
        })
    }

    pub fn next_page<T>(&self, page: &KnowledgePage<T>) -> Option<Self> {
        page.next_cursor.clone().map(|cursor| Self {
            limit: self.limit,
            cursor: Some(cursor),
        })
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn cursor(&self) -> Option<&KnowledgeCursor> {
        self.cursor.as_ref()
    }
}

fn validate_limit(limit: usize) -> Result<usize, StoreError> {
    if (1..=MAX_KNOWLEDGE_PAGE_SIZE).contains(&limit) {
        Ok(limit)
    } else {
        Err(if limit == 0 {
            StoreError::Invalid
        } else {
            StoreError::LimitExceeded
        })
    }
}
