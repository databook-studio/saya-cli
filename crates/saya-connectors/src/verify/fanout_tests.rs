//! Tests for the fan-out probe builder.

#[path = "fanout_tests/ctes.rs"]
mod ctes;
#[path = "fanout_tests/emission.rs"]
mod emission;
#[path = "fanout_tests/refusals.rs"]
mod refusals;
#[path = "fanout_tests/safety.rs"]
mod safety;

use saya_types::SqlDialect;

/// Postgres accepts the most syntax (catalog-qualified names, CTEs, SELECT
/// INTO), so it is the default dialect for these tests unless a case is
/// dialect-specific.
const D: SqlDialect = SqlDialect::Postgres;
