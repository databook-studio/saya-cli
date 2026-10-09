use std::sync::Arc;

use super::{EngineEventSink, EngineSinkError};

impl EngineEventSink {
    /// Takes the last failure the sink held — one it had no error channel to
    /// return through, or a store failure `record` surfaced and also held.
    /// `None` when nothing failed.
    pub fn take_diagnostic(&self) -> Option<Arc<EngineSinkError>> {
        self.diagnostic
            .lock()
            .expect("engine sink diagnostic lock")
            .take()
    }

    pub(in crate::engine) fn hold_diagnostic(&self, error: EngineSinkError) {
        self.hold_diagnostic_arc(Arc::new(error));
    }

    pub(in crate::engine) fn hold_diagnostic_arc(&self, error: Arc<EngineSinkError>) {
        let mut diagnostic = self.diagnostic.lock().expect("engine sink diagnostic lock");
        if diagnostic.is_none() {
            *diagnostic = Some(error);
        }
    }
}
