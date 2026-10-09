//! Tests for the DuckDB native bind path: the concrete `duckdb::types::Value`
//! encodings, the decimal-precision refusal, the `?` marker rewrite, and
//! end-to-end execution through the connector with every parameter type.
//!
//! Seeding uses a raw DuckDB connection on a temp file: the connector's
//! read-only gate must keep refusing writes, so the fixture is created
//! outside it — the same approach the SQLite tests take.

#[path = "bind_tests/execution.rs"]
mod execution;
#[path = "bind_tests/native_encodings.rs"]
mod native_encodings;
