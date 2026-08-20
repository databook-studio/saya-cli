//! Builds and records tool observations from a SQL statement and its result.
//!
//! Kept separate from the collector so the recording concern (extracting names
//! and outcome metadata) is testable without the executor. An observation never
//! copies SQL or result values — only the fields `ToolObservation` carries.

use saya_connectors::sql_references;
use saya_types::{ProfileIdentity, SqlDialect};
use serde_json::Value;

use super::observations::{ObservationLog, ObservationOutcome, ToolObservation};

impl ObservationLog {
    /// Records a query-tool observation. `profile_id` is the resolved
    /// connection's identity (None when the connection has no identity, e.g. a
    /// test connector). `sql` is the statement as the model wrote it. `result`
    /// is the tool's serialized result on success; on failure it is `None`.
    pub(crate) fn record_query(
        &self,
        tool: &str,
        sql: &str,
        dialect: SqlDialect,
        profile_id: Option<&str>,
        result: Option<&Value>,
    ) {
        let refs = sql_references(sql, dialect).unwrap_or_default();
        let (objects, columns, references_partial) = (refs.objects, refs.columns, refs.partial);
        let (outcome, row_count, truncated) = match result {
            Some(value) => (
                ObservationOutcome::Succeeded,
                value
                    .get("row_count")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize),
                value.get("truncated").and_then(Value::as_bool),
            ),
            // A failure records the objects the SQL named (if it parsed) and
            // nothing else — no error string, no row count, no truncation.
            None => (ObservationOutcome::Failed, None, None),
        };
        self.record(ToolObservation {
            tool: tool.into(),
            outcome,
            profile: profile_id.and_then(parse_identity),
            objects,
            columns,
            row_count,
            truncated,
            references_partial,
        });
    }

    /// Records a schema-discovery observation. It touches no specific table, so
    /// it carries no objects or columns.
    pub(crate) fn record_schema(&self, tool: &str, profile_id: Option<&str>, succeeded: bool) {
        self.record(ToolObservation {
            tool: tool.into(),
            outcome: if succeeded {
                ObservationOutcome::Succeeded
            } else {
                ObservationOutcome::Failed
            },
            profile: profile_id.and_then(parse_identity),
            objects: Vec::new(),
            columns: Vec::new(),
            row_count: None,
            truncated: None,
            references_partial: false,
        });
    }
}

fn parse_identity(value: &str) -> Option<ProfileIdentity> {
    // The registry stores the validated `p-<64 hex>` identity as a string; a
    // malformed value means no profile to attribute, so degrade to None rather
    // than fail the tool over an observation.
    ProfileIdentity::parse(value).ok()
}
