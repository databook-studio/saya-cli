//! The result one successful query handed the front: the rows the model saw, or
//! the refusal that kept them from it.
//!
//! Presentation-free like the rest of the view model: the preview table (T6)
//! reads these facts, and nothing here names a column, a glyph or a width.

use std::sync::Arc;

use saya_types::{ExecutionEvidence, QueryResult};

use crate::agent::tools::{CaptureEvent, CaptureRefusalReason};

/// The result one successful query handed the front: the rows the model saw,
/// or the refusal that kept them from it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResultPreview {
    /// The connection that actually ran the query.
    pub(crate) connection: String,
    pub(crate) sql: String,
    pub(crate) result: QueryResult,
    /// The execution the rows came from: the provenance the Markdown report
    /// states. A refused capture has none — nothing ran. Shared, because a
    /// preview is cloned with the transcript it sits in.
    pub(crate) evidence: Option<Arc<ExecutionEvidence>>,
    /// Why the capture was refused: a preview with no rows says so, and never
    /// pretends the result was empty.
    pub(crate) refusal: Option<CaptureRefusalReason>,
}

impl ResultPreview {
    /// The preview of one capture event, held or refused.
    pub(crate) fn from_capture(event: &CaptureEvent) -> Self {
        match event {
            CaptureEvent::Captured(capture) => Self {
                connection: capture.connection.clone(),
                sql: capture.sql.clone(),
                result: capture.result.clone(),
                // The capture's own facts time and name the execution, so the
                // report written from this entry is the report the capture
                // would have written.
                evidence: Some(Arc::new(crate::interactive::tui::capture::agent_evidence(
                    capture,
                ))),
                refusal: None,
            },
            CaptureEvent::Refused {
                sql,
                connection,
                reason,
            } => Self {
                connection: connection.clone(),
                sql: sql.clone(),
                result: QueryResult::empty(sql),
                evidence: None,
                refusal: Some(*reason),
            },
        }
    }
}
