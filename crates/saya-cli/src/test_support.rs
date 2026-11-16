//! Test-only support that belongs to no single surface.

use std::path::Path;
use std::time::{Duration, Instant};

/// Removes a fixture tree after the SQLite pools inside it have been dropped.
///
/// A pool returns a leased connection through a task it spawns, and Windows
/// keeps the database file open until that task drops the connection. The wait
/// between attempts is therefore an async sleep, not a blocking one: these
/// tests run on a current-thread runtime, and blocking the only thread would
/// starve the very task the retry is waiting for. Retrying for a bounded time
/// still asserts the tree goes once the pool lets it go; it never skips the
/// assertion.
pub(crate) async fn remove_dir_all_settling(root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match std::fs::remove_dir_all(root) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) if Instant::now() >= deadline => {
                panic!("synthetic fixture is not removed: {error}");
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
}
