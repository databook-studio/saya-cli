//! Host-memory copy of exactly the redacted bytes accepted by writer calls.
use super::output::{OUTPUT_STORE_CAP_BYTES, TRUNCATION_MARKER};
use std::sync::Arc;
const MAX_SNAPSHOT_BYTES: usize = OUTPUT_STORE_CAP_BYTES + TRUNCATION_MARKER.len();

pub(super) struct CapturedWrites {
    stored_bytes: u64,
    bytes: Option<Vec<u8>>,
}
impl CapturedWrites {
    pub(super) fn new() -> Self {
        Self {
            stored_bytes: 0,
            bytes: Some(Vec::with_capacity(MAX_SNAPSHOT_BYTES)),
        }
    }
    /// Call only with the exact successful prefix returned by the file writer.
    pub(super) fn record(&mut self, bytes: &[u8]) {
        self.stored_bytes = self.stored_bytes.saturating_add(bytes.len() as u64);
        if let Some(kept) = &mut self.bytes {
            if kept.len().saturating_add(bytes.len()) <= MAX_SNAPSHOT_BYTES {
                kept.extend_from_slice(bytes);
            } else {
                self.bytes = None;
            }
        }
    }
    pub(super) fn stored_bytes(&self) -> u64 {
        self.stored_bytes
    }
    pub(super) fn into_bytes(self) -> Option<Arc<[u8]>> {
        self.bytes.map(Arc::from)
    }
}
