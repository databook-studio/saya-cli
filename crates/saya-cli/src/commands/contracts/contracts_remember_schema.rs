//! The cached-schema check `contracts remember` runs before storing, and the
//! "object not in the cached schema" message it raises on refusal. Split out
//! of `contracts_write.rs` so the write commands stay under the soft file-size
//! cap and the schema-resolution logic — the part that fixes the two
//! remember-time symptoms — reads in one place.
//!
//! `remember` loads the cached schema via the shared `super::cached_schema`
//! helper (the same one `list`/`show` use) and asks [`resolved_against`] what
//! to do: store the real digest against a found table, refuse an absent
//! object, or fall back to the unobserved sentinel when there is no schema to
//! check against.

use super::EXIT_CONTRACT_ERROR;
use crate::commands::output::failure_message;
use crate::render::RenderFormat;
use saya_types::{DatabaseObjectRef, SchemaFingerprint, SchemaTree, Table};

/// What the cached schema says about `object`. `Found` carries the live table
/// to fingerprint and snapshot against; `Absent` refuses; `NoSchema` keeps the
/// unobserved-sentinel behaviour. An empty cached tree (the no-op sentinel a
/// fresh store writes, or a never-populated cache) is `NoSchema`, not `Absent`
/// — it carries no real schema information, so refusing every object as
/// "unknown" would break `remember` before a first refresh. Mirrors
/// `classify::is_stale` in the import path, which treats an empty `databases`
/// list as "not stale".
pub(super) enum SchemaCheck<'a> {
    Found(&'a Table),
    Absent,
    NoSchema,
}

/// Classify `object` against the optional cached `schema`. See [`SchemaCheck`].
pub(super) fn resolved_against<'a>(
    cached: &'a Option<SchemaTree>,
    object: &DatabaseObjectRef,
) -> SchemaCheck<'a> {
    let Some(schema) = cached else {
        return SchemaCheck::NoSchema;
    };
    if schema.databases.is_empty() {
        return SchemaCheck::NoSchema;
    }
    match schema.find_table(object.catalog(), object.schema(), object.object()) {
        Some(table) => SchemaCheck::Found(table),
        None => SchemaCheck::Absent,
    }
}

/// The real digest of `table`, what a `Found` claim stores so it reads
/// `current` against a matching cache instead of `needs_review`.
pub(super) fn fingerprint_of(table: &Table) -> SchemaFingerprint {
    SchemaFingerprint::of_table(saya_types::DatabaseObjectKind::Table, table)
}

/// Emit the "object not in the cached schema" refusal and return the contract
/// error exit code. The caller has already decided the object is `Absent`.
pub(super) fn refuse_unknown(
    object: &DatabaseObjectRef,
    profile_name: &str,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    failure_message(
        EXIT_CONTRACT_ERROR,
        unknown_object_message(object, profile_name),
        format,
    )
}

/// The "object not in the cached schema" message. The object name is safe to
/// echo: `parse_qualified` + `DatabaseObjectRef::new` already validated it is
/// three non-empty, length-bounded, control-char-free identifiers before this
/// point. `profile_name` is the config name (not the opaque identity) the read
/// commands already render; the opaque identity never appears here.
fn unknown_object_message(object: &DatabaseObjectRef, profile_name: &str) -> String {
    format!(
        "no object {} in the cached schema for profile {}; run `connection schema {profile_name} --refresh` and retry",
        object.qualified_name(),
        profile_name,
    )
}
