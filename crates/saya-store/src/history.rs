use crate::{SessionHistoryQuery, SessionSummary, StoreError};
use std::{fs, num::NonZeroUsize, path::Path, time::UNIX_EPOCH};

/// The largest page a caller may request. Directory enumeration retains at
/// most one extra summary to determine whether a continuation exists.
pub const MAX_SESSION_HISTORY_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionHistoryLimit(NonZeroUsize);

impl SessionHistoryLimit {
    pub fn get(self) -> usize {
        self.0.get()
    }
}

impl TryFrom<usize> for SessionHistoryLimit {
    type Error = StoreError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        let value = NonZeroUsize::new(value).ok_or(StoreError::Invalid)?;
        if value.get() > MAX_SESSION_HISTORY_PAGE_SIZE {
            return Err(StoreError::LimitExceeded);
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHistoryCursor {
    modified_unix_ns: u128,
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHistoryPage {
    pub entries: Vec<SessionSummary>,
    pub next_cursor: Option<SessionHistoryCursor>,
}

struct HistoryEntry {
    summary: SessionSummary,
    modified_unix_ns: u128,
}

impl SessionHistoryPage {
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some()
    }
}

pub(crate) fn list(
    root: &Path,
    query: &SessionHistoryQuery,
) -> Result<SessionHistoryPage, StoreError> {
    let entries = match fs::read_dir(root) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SessionHistoryPage {
                entries: Vec::new(),
                next_cursor: None,
            });
        }
        Err(_) => return Err(StoreError::unavailable()),
    };
    let limit = query.limit();
    let mut history = Vec::with_capacity(limit.saturating_add(1));
    for entry in entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
    {
        // The session id is the file stem (save writes `<id>.json`), so listing
        // needs neither to read nor to deserialize the whole conversation — only
        // the name and the modification time. This keeps listing O(sessions),
        // independent of session size.
        let path = entry.path();
        let Some(id) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        let modified_unix_ns = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        if query.cursor().is_some_and(|cursor| {
            modified_unix_ns > cursor.modified_unix_ns
                || (modified_unix_ns == cursor.modified_unix_ns
                    && id.as_str() >= cursor.id.as_str())
        }) {
            continue;
        }
        history.push(HistoryEntry {
            summary: SessionSummary {
                id,
                modified_unix_ms: modified_unix_ns / 1_000_000,
            },
            modified_unix_ns,
        });
        history.sort_unstable_by(compare_entry);
        if history.len() > limit.saturating_add(1) {
            history.pop();
        }
    }
    history.sort_unstable_by(compare_entry);
    let next_cursor = (history.len() > limit).then(|| {
        let entry = &history[limit - 1];
        SessionHistoryCursor {
            modified_unix_ns: entry.modified_unix_ns,
            id: entry.summary.id.clone(),
        }
    });
    Ok(SessionHistoryPage {
        entries: history
            .into_iter()
            .take(limit)
            .map(|entry| entry.summary)
            .collect(),
        next_cursor,
    })
}

fn compare_entry(left: &HistoryEntry, right: &HistoryEntry) -> std::cmp::Ordering {
    right
        .modified_unix_ns
        .cmp(&left.modified_unix_ns)
        .then_with(|| right.summary.id.cmp(&left.summary.id))
}
