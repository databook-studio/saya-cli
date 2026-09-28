//! Stale detection, breaking, and the time plumbing for the repository
//! lock (A1).
//!
//! A lock older than the staleness horizon is treated as abandoned and is
//! broken by atomically renaming it aside — never deleted — so the break
//! never destroys evidence. The moved file's age is re-verified after the
//! rename and a file that turns out fresh is renamed back untouched: a
//! lock released and re-claimed inside the break window keeps its claim.
//! A lock whose age cannot be determined at all is live: fail closed,
//! never break what was not verified stale.

use super::lock::LockContents;
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Breaks the lock at `path` when it is verified stale: renamed aside, the
/// moved file's age is checked again, and a file that turned out fresh is
/// renamed back untouched. Reports whether the break succeeded.
pub(super) fn break_if_stale(path: &Path, stale_after: Duration) -> bool {
    if !is_stale(path, stale_after) {
        return false;
    }
    let aside = path.with_file_name(format!(".lock.stale-{}", now_unix_ms()));
    if std::fs::rename(path, &aside).is_err() {
        return false;
    }
    if is_stale(&aside, stale_after) {
        return true;
    }
    let _ = std::fs::rename(&aside, path);
    false
}

/// Whether the lock file's age has reached `stale_after`: from its recorded
/// acquisition time, falling back to the file's modification time when the
/// contents cannot be parsed (a lock abandoned before its contents were
/// written). A lock whose age cannot be determined at all is live.
fn is_stale(path: &Path, stale_after: Duration) -> bool {
    let recorded = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<LockContents>(&bytes).ok())
        .map(|contents| contents.acquired_unix_ms);
    let age = match recorded {
        Some(acquired_unix_ms) => age_from_unix_ms(acquired_unix_ms),
        None => std::fs::metadata(path).ok().and_then(|metadata| {
            let modified_ms = metadata
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_millis() as i64;
            age_from_unix_ms(modified_ms)
        }),
    };
    age.is_some_and(|age| age >= stale_after)
}

/// The duration since `unix_ms`; `None` for a future timestamp (clock
/// skew) — treated as live by the caller.
fn age_from_unix_ms(unix_ms: i64) -> Option<Duration> {
    let now = now_unix_ms();
    (unix_ms <= now).then(|| Duration::from_millis((now - unix_ms) as u64))
}

/// The current time as unix milliseconds.
pub(super) fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or_default()
}
