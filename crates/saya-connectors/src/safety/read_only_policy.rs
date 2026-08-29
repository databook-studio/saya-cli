//! Per-backend function denylists for the read-only safety layer, plus the
//! matchers that decide whether a parsed function or relation name is denied.
//!
//! There are two matchers with different jobs:
//!
//! - [`denied_function`] compares *each identifier part* (lowercased, unquoted)
//!   so that schema qualification (`pg_catalog.pg_read_file`) or requoting
//!   (`` `nextval` ``) cannot bypass the guard. Prefix rules apply per part as
//!   well, so `my_schema.system$get_presigned_url` still hits the `system$`
//!   rule. This matcher is applied to every function reference in the tree — a
//!   scalar function call, a table function in `FROM` (`SELECT * FROM fn(...)`),
//!   and a `LATERAL fn(...)` table factor — wherever it sits.
//! - [`denied_relation`] compares the *whole* dotted name and is applied only
//!   to plain table references (a `FROM` table with no call arguments). It is
//!   deliberately fail-closed: a table literally named like a denied function
//!   (e.g. `nextval`) stays blocked. Because it matches the whole name, a
//!   schema-qualified *plain* table (`public.nextval`) is not caught here — but
//!   such a name is a table, not a function call, and the function-shaped
//!   bypasses are closed by [`denied_function`] above.

use sqlparser::ast::ObjectName;

pub(super) struct BackendPolicy {
    pub denied_functions: &'static [&'static str],
    pub denied_prefixes: &'static [&'static str],
}

const COMMON_DENIED_FUNCTIONS: &[&str] = &["nextval", "setval"];

const DUCKDB_DENIED_FUNCTIONS: &[&str] = &[
    "read_csv",
    "read_csv_auto",
    "read_json",
    "read_json_auto",
    "read_parquet",
    "read_text",
    "sqlite_scan",
    "glob",
    "metadata",
];

const SQLITE_DENIED_FUNCTIONS: &[&str] = &["load_extension", "readfile", "writefile"];

const SNOWFLAKE_DENIED_FUNCTIONS: &[&str] =
    &["get_presigned_url", "build_scoped_file_url", "directory"];

const SNOWFLAKE_DENIED_PREFIXES: &[&str] = &["@", "system$"];

const POSTGRES_DENIED_FUNCTIONS: &[&str] = &[
    "set_config",
    "setseed",
    "pg_advisory_lock",
    "pg_advisory_xact_lock",
    "pg_advisory_shared_lock",
    "pg_advisory_unlock",
    "pg_advisory_unlock_all",
    "pg_terminate_backend",
    "pg_cancel_backend",
    "pg_reload_conf",
    "pg_rotate_logfile",
    "pg_read_file",
    "pg_read_binary_file",
    "pg_ls_dir",
    "pg_stat_file",
    "lo_import",
    "lo_export",
    "lo_open",
    "lo_get",
    "lo_put",
    "loread",
    "lowrite",
    "pg_sleep",
    "pg_sleep_for",
    "pg_sleep_until",
    "pg_create_restore_point",
    "pg_logical_emit_message",
];

const POSTGRES_DENIED_PREFIXES: &[&str] = &["dblink"];

const MYSQL_DENIED_FUNCTIONS: &[&str] = &[
    "get_lock",
    "release_lock",
    "release_all_locks",
    "load_file",
    "sleep",
    "sys_exec",
    "sys_eval",
];

pub(super) const POSTGRES_POLICY: BackendPolicy = BackendPolicy {
    denied_functions: POSTGRES_DENIED_FUNCTIONS,
    denied_prefixes: POSTGRES_DENIED_PREFIXES,
};

pub(super) const MYSQL_POLICY: BackendPolicy = BackendPolicy {
    denied_functions: MYSQL_DENIED_FUNCTIONS,
    denied_prefixes: &[],
};

pub(super) const DUCKDB_POLICY: BackendPolicy = BackendPolicy {
    denied_functions: DUCKDB_DENIED_FUNCTIONS,
    denied_prefixes: &[],
};

pub(super) const SQLITE_POLICY: BackendPolicy = BackendPolicy {
    denied_functions: SQLITE_DENIED_FUNCTIONS,
    denied_prefixes: &[],
};

pub(super) const SNOWFLAKE_POLICY: BackendPolicy = BackendPolicy {
    denied_functions: SNOWFLAKE_DENIED_FUNCTIONS,
    denied_prefixes: SNOWFLAKE_DENIED_PREFIXES,
};

/// True when any identifier part of a *function* reference matches a denied
/// name or prefix. Applied to scalar function calls, table functions in `FROM`
/// (a `Table` factor that carries call arguments), and `LATERAL`/function
/// table factors — so schema qualification (`pg_catalog.pg_read_file`) cannot
/// hide a denied name. Plain table references (no call arguments) keep the
/// stricter whole-name [`denied_relation`] check, so a table literally named
/// like a denied function (e.g. `nextval`) stays blocked.
pub(super) fn denied_function(name: &ObjectName, policy: &BackendPolicy) -> bool {
    name.0.iter().any(|ident| {
        let part = ident.value.to_ascii_lowercase();
        COMMON_DENIED_FUNCTIONS.contains(&part.as_str())
            || policy.denied_functions.contains(&part.as_str())
            || policy
                .denied_prefixes
                .iter()
                .any(|prefix| part.starts_with(prefix))
    })
}

/// Whole-name check used for relations (tables/stages), preserving the
/// historical fail-closed behaviour including `@stage` prefixes.
pub(super) fn denied_relation(name: &ObjectName, policy: &BackendPolicy) -> bool {
    let name = name.to_string().trim_matches('"').to_ascii_lowercase();
    COMMON_DENIED_FUNCTIONS.contains(&name.as_str())
        || policy.denied_functions.contains(&name.as_str())
        || policy
            .denied_prefixes
            .iter()
            .any(|prefix| name.starts_with(prefix))
}
