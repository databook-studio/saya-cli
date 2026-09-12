//! Shared public contracts for SAYA CLI.

mod contract;
mod dialect;
mod error;
mod profile;
mod query;
mod redaction;
mod run;
mod schema;

pub use contract::{
    BindingValidity, CLAIM_PAYLOAD_VERSION, Cardinality, ClaimId, ClaimOrigin, ClaimPayload,
    ClaimStatus, ColumnRequirement, ColumnRole, ContractError, DatabaseObjectKind,
    DatabaseObjectRef, DateGrain, FINGERPRINT_VERSION, KnowledgeSlot, KnowledgeState,
    MAX_MULTI_SLOT_VALUES, MAX_NAME_CHARS, MAX_PROFILE_NAME_CHARS, MAX_REFERENCED_COLUMNS,
    MAX_TEXT_CHARS, MAX_TIMEZONE_CHARS, OutputStyle, PreferenceScope, PreferenceValue,
    ProfileIdentity, ReferencedColumn, SchemaBinding, SchemaFingerprint, ScopeRequirement, Scoped,
    SlotCardinality, SlotError, SlotParseError, SlotValues, is_numeric_type, is_temporal_type,
    validate_table,
};
pub use dialect::SqlDialect;
pub use error::ConnectionError;
pub use profile::{DatabaseProfile, MySqlSslMode, PostgresSslMode, SecretRef, SnowflakeAuth};
pub use query::{QueryRequest, QueryResult};
pub use redaction::{CREDENTIAL_ENV_PREFIX, redact};
pub use run::{
    Budgets, Capabilities, Deliverable, DeliverableArtifact, Destination, EndpointBindings,
    FetchScope, InterpreterScope, MAX_BUDGET_ENDPOINTS, MAX_ENDPOINT_BINDINGS,
    MAX_FETCH_DESTINATIONS, MAX_GOAL_BYTES, MAX_OUTPUT_HINTS, MAX_PLAN_STEPS, MAX_RUNNER_PROGRAMS,
    MAX_STEP_CREDENTIALS, OutputHint, PauseReason, RunContractError, RunEvent, RunFailureCode,
    RunId, RunPlan, RunSpec, RunnerScope, StepSpec, is_bare_name, is_name_shaped,
    is_refused_runner_program,
};
pub use schema::{Column, Database, ForeignKey, Schema, SchemaTree, Table};
