//! Exemplar property test for the read-only safety layer.
//!
//! Independent oracle statement-class verification:
//! This test suite proves that any SQL statement accepted by `prepare_postgres_sql` or
//! `prepare_sqlite_sql` is converted into a string that belongs strictly to a read-only
//! statement class (`Statement::Query`, `Explain` wrapping a `Query`, or a `Show*` variant),
//! verified via an independent parser-based oracle implemented in this test module.
//!
//! LIMIT: This oracle proves statement-CLASS read-only safety. It does not prove that an
//! accepted function (e.g. custom UDF) is side-effect free; function-level side-effects
//! are enforced at the DB session level and via read-only connection roles (see SECURITY.md).

#[path = "read_only_property/oracle.rs"]
mod oracle;
#[cfg(test)]
#[path = "read_only_property/oracle_cases.rs"]
mod oracle_cases;
#[path = "read_only_property/property_cases.rs"]
mod property_cases;
