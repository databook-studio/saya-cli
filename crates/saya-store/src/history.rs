use crate::{SessionSummary, StoreError};
use std::{fs, path::Path, time::UNIX_EPOCH};

pub(crate) fn list(root: &Path) -> Result<Vec<SessionSummary>, StoreError> {
    let entries = match fs::read_dir(root) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(StoreError::unavailable()),
    };
    let mut history = Vec::new();
    for entry in entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
    {
        let path = entry.path();
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(session) = serde_json::from_str::<crate::RedactedSession>(&content) else {
            continue;
        };
        let modified_unix_ms = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        history.push(SessionSummary {
            id: session.id,
            modified_unix_ms,
        });
    }
    history.sort_by(|left, right| {
        right
            .modified_unix_ms
            .cmp(&left.modified_unix_ms)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(history)
}
