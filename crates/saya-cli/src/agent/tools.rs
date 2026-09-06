//! Agent-facing database tool definitions, execution, and display helpers.

#[cfg(test)]
use crate::connection::{ConnectionEntry, ConnectionRegistry};

mod contract_tools;
mod database_tools;
mod executor;
mod sql_format;
mod tool_calls;

pub(crate) use database_tools::DatabaseTools;
// Re-exported through `tools` (not the private `database_tools` module) so the
// agent runtime's learning wiring and tests can reach the observation types.
// `ToolObservation` is consumed only by tests; the others by `agent::learning`.
// `OverrideLog` is drained by the runtime to emit `KnowledgeOverridden` (A1).
#[allow(unused_imports)]
pub(crate) use database_tools::{
    DrainedObservations, ObservationLog, ObservationOutcome, OverrideLog, ToolObservation,
};
#[allow(unused_imports)]
pub(crate) use sql_format::format_sql;
#[allow(unused_imports)]
pub(crate) use tool_calls::{SqlCall, sql_tool_call, tool_call_detail};

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tools/contract_tools_tests.rs"]
mod contract_tools_tests;

#[cfg(test)]
#[path = "tools/format_tests.rs"]
mod format_tests;

#[cfg(test)]
#[path = "tools/observations_tests.rs"]
mod observations_tests;

#[cfg(test)]
#[path = "tools/result_shape_tests.rs"]
mod result_shape_tests;

#[cfg(test)]
#[path = "tools/column_health_tests.rs"]
mod column_health_tests;

#[cfg(test)]
#[path = "tools/join_check_tests.rs"]
mod join_check_tests;
