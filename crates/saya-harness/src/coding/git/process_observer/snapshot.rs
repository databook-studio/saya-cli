//! The bounded read view of the receipts an observer retained for its session.

use super::receipt::GitProcessReceipt;

#[derive(Debug, Clone)]
pub struct GitProcessSnapshot {
    pub(super) service_identity: String,
    pub(super) receipts: Vec<GitProcessReceipt>,
    pub(super) omitted: u64,
    pub(super) omitted_bytes: u64,
    pub(super) incomplete: bool,
}

impl GitProcessSnapshot {
    pub fn service_identity(&self) -> &str {
        &self.service_identity
    }
    pub fn receipts(&self) -> &[GitProcessReceipt] {
        &self.receipts
    }
    pub fn omitted(&self) -> u64 {
        self.omitted
    }
    pub fn omitted_bytes(&self) -> u64 {
        self.omitted_bytes
    }
    pub fn complete(&self) -> bool {
        self.omitted == 0 && !self.incomplete
    }
}
