use saya_harness::coding::extensions::{
    ExtensionError, ExtensionProcessEvent, ExtensionProcessObserver,
};
use saya_types::{JobId, ManagedCheckoutId};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
pub(super) struct ProcessObserver {
    records: PathBuf,
    job: JobId,
    checkout: ManagedCheckoutId,
    epoch: u64,
    operation: u64,
}
impl ProcessObserver {
    pub(super) fn new(ops: &super::checks::ChildCodingOperations) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            records: ops.records.clone(),
            job: ops.job.clone(),
            checkout: ops.checkout.clone(),
            epoch: ops.authority.revocation_epoch(),
            operation: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }
}
impl ExtensionProcessObserver for ProcessObserver {
    fn record(&self, event: ExtensionProcessEvent) -> Result<(), ExtensionError> {
        let fact = match event {
            ExtensionProcessEvent::Spawned { pid } => json!({"phase":"spawned","pid":pid}),
            ExtensionProcessEvent::Settled(receipt) => {
                json!({"phase":"settled","pid":receipt.pid,"exit_code":receipt.exit_code,"signal":receipt.signal,"joined":receipt.joined,"group_kill_requested":receipt.group_kill_requested,"group_kill_delivered":receipt.group_kill_delivered,"escaped_descendants_proven_absent":receipt.escaped_descendants_proven_absent})
            }
            ExtensionProcessEvent::Uncertain { pid } => json!({"phase":"uncertain","pid":pid}),
        };
        let value = json!({"job_id":self.job,"checkout_id":self.checkout,"epoch":self.epoch,"operation":self.operation,"fact":fact});
        super::context::append_line(
            &self
                .records
                .join(format!("extensions-{}.jsonl", self.checkout.as_str())),
            &serde_json::to_vec(&value).map_err(|_| ExtensionError::Process)?,
            1024 * 1024,
        )
        .map_err(|_| ExtensionError::Process)
    }
}
