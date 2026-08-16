//! Shared public contracts for SAYA CLI.

mod budget;
mod contract;
mod dialect;
mod error;
mod profile;
mod query;
mod schema;

pub use budget::MAX_MESSAGE_BYTES;
pub use contract::{
    CLAIM_PAYLOAD_VERSION, Cardinality, ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus,
    ColumnRole, ContractError, DatabaseObjectKind, DatabaseObjectRef, DateGrain,
    FINGERPRINT_VERSION, KnowledgeSlot, KnowledgeState, MAX_MULTI_SLOT_VALUES, MAX_NAME_CHARS,
    MAX_PROFILE_NAME_CHARS, MAX_REFERENCED_COLUMNS, MAX_TEXT_CHARS, MAX_TIMEZONE_CHARS,
    OutputStyle, PreferenceScope, PreferenceValue, ProfileIdentity, ReferencedColumn,
    SchemaFingerprint, ScopeRequirement, Scoped, SlotCardinality, SlotError, SlotParseError,
    SlotValues,
};
pub use dialect::SqlDialect;
pub use error::ConnectionError;
pub use profile::{DatabaseProfile, MySqlSslMode, PostgresSslMode, SecretRef, SnowflakeAuth};
pub use query::{QueryRequest, QueryResult};
pub use schema::{Column, Database, Schema, SchemaTree, Table};
