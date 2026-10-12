//! Per-`(job, checkout)` create serialization: weakly keyed locks pruned on
//! every insert, so the map's bound is the number of in-flight keys.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, Weak},
};

/// Per-(job, checkout) create serialization: the whole
/// read–effect–outcome run for one id holds its lock, so a concurrent
/// same-id call observes the settled record instead of racing the intent
/// append. Cross-process serialization is not provided; the store itself
/// still enforces one durable outcome. Entries are weak and pruned on
/// every insert, so the map's bound is the number of in-flight keys.
type CreateLockMap = HashMap<(String, String), Weak<tokio::sync::Mutex<()>>>;

static CREATE_LOCKS: LazyLock<Mutex<CreateLockMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn create_lock(job_id: &str, checkout_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let key = (job_id.to_owned(), checkout_id.to_owned());
    let mut locks = CREATE_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.retain(|_, lock| lock.upgrade().is_some());
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        lock
    } else {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }
}
