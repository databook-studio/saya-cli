//! The session store contract: the trait stores implement, and the query
//! and summary types its history surface is phrased in.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::StoreError;
use crate::history::{SessionHistoryCursor, SessionHistoryLimit, SessionHistoryPage};
use crate::session::RedactedSession;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub modified_unix_ms: u128,
}

/// A bounded request for one page of saved sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHistoryQuery {
    limit: SessionHistoryLimit,
    cursor: Option<SessionHistoryCursor>,
}

impl SessionHistoryQuery {
    pub fn first_page(limit: usize) -> Result<Self, StoreError> {
        Ok(Self {
            limit: SessionHistoryLimit::try_from(limit)?,
            cursor: None,
        })
    }

    pub fn next_page(&self, page: &SessionHistoryPage) -> Option<Self> {
        page.next_cursor.clone().map(|cursor| Self {
            limit: self.limit,
            cursor: Some(cursor),
        })
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit.get()
    }

    pub(crate) fn cursor(&self) -> Option<&SessionHistoryCursor> {
        self.cursor.as_ref()
    }
}

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn save(&self, session: RedactedSession) -> Result<(), StoreError>;
    async fn load(&self, id: &str) -> Result<Option<RedactedSession>, StoreError>;
    async fn most_recent(&self) -> Result<Option<RedactedSession>, StoreError>;
    async fn history(&self, query: SessionHistoryQuery) -> Result<SessionHistoryPage, StoreError>;
}
