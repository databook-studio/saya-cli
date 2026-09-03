//! Shared public contracts for SAYA CLI.

mod contract;
mod dialect;
mod error;
mod profile;
mod query;
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
pub use schema::{Column, Database, Schema, SchemaTree, Table};
