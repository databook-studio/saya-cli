use saya_store::SqliteStateStore;
use saya_types::{CodingEventKind, JobId};
use std::{path::PathBuf, time::Duration};
use tokio::time::Instant;

/// The shared working checkout W's creation intent: the agent-less `work-…`
/// checkout the helper's own mutation lands in (helpers share W).
pub(super) async fn root_checkout_intent(
    store: &SqliteStateStore,
    job_id: &JobId,
    deadline: Instant,
) -> Option<(String, PathBuf)> {
    loop {
        let page = store.list_coding_events(job_id, None, 100).await.ok()?;
        if let Some(checkout) = page.events.iter().find_map(|event| match event.event() {
            CodingEventKind::CheckoutCreateIntent { checkout, .. }
                if checkout.managed_checkout_id().as_str().starts_with("work-") =>
            {
                Some(checkout)
            }
            _ => None,
        }) {
            return Some((
                checkout.managed_checkout_id().as_str().to_owned(),
                PathBuf::from(checkout.managed_path().as_str()),
            ));
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(super) async fn fail_and_join<T>(
    task: &mut tokio::task::JoinHandle<T>,
    unblock: impl FnOnce(),
    cancel: impl FnOnce(),
    message: &str,
) -> ! {
    unblock();
    cancel();
    if tokio::time::timeout(Duration::from_secs(10), &mut *task)
        .await
        .is_err()
    {
        task.abort();
        let _ = task.await;
        panic!("{message}; canceled job required abort and join");
    }
    panic!("{message}");
}
