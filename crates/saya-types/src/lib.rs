//! Shared public contracts for SAYA CLI.

mod dialect;
mod error;
mod profile;
mod query;
mod schema;

pub use dialect::SqlDialect;
pub use error::ConnectionError;
pub use profile::{DatabaseProfile, SecretRef, SnowflakeAuth};
pub use query::{QueryRequest, QueryResult};
pub use schema::{Column, Database, Schema, SchemaTree, Table};
