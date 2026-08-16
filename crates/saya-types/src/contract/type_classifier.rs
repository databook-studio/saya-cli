//! Semantic type classification for connector data types across SQL dialects.
//!
//! Rather than forcing every database connector to map its proprietary data type
//! spellings into an exhaustive enum (which frequently breaks on minor type widenings
//! or dialect aliases), SAYA uses semantic type classifiers.

/// Classifies whether a connector's raw data type string represents a temporal
/// value (date, time, timestamp, or datetime).
///
/// We classify types by semantic substring matching rather than exact type enum
/// matching because database dialects express temporal representations with
/// varied spellings, precision parameters, and timezone suffixes (e.g. Postgres
/// `timestamp with time zone` and `timestamptz`, MySQL `datetime`, Snowflake
/// `TIMESTAMP_NTZ`/`LTZ`/`TZ`, DuckDB `time`). Substring matching also allows
/// harmless schema evolution (such as widening `TIMESTAMP` to `TIMESTAMPTZ`)
/// without falsely invalidating user-taught temporal knowledge like default time.
pub fn is_temporal_type(data_type: &str) -> bool {
    let lower = data_type.trim().to_ascii_lowercase();
    lower.contains("time")
        || lower.contains("date")
        || lower.contains("timestamp")
        || lower.contains("datetime")
}

/// Classifies whether a connector's raw data type string represents a numeric
/// value (integers, floating point, decimals, serials, and numbers).
///
/// We classify types by semantic substring matching because database engines
/// use dialect-specific naming and parameterized widths (e.g. Postgres
/// `bigserial` / `double precision`, MySQL `int(11)` / `decimal(10,2)`,
/// Snowflake `NUMBER(38,0)`, DuckDB `HUGEINT`, SQLite `real`). Facts with
/// numeric requirements (such as measure roles) remain valid when schemas evolve
/// across numeric precision widenings (e.g. `int` to `bigint`).
pub fn is_numeric_type(data_type: &str) -> bool {
    let lower = data_type.trim().to_ascii_lowercase();
    lower.contains("int")
        || lower.contains("float")
        || lower.contains("double")
        || lower.contains("decimal")
        || lower.contains("numeric")
        || lower.contains("real")
        || lower.contains("number")
        || lower.contains("serial")
}
