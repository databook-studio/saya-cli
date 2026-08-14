//! Rendering confirmed claims to the discovered-shape TOML (slice 6b).
//!
//! The file shape is exactly what 6a's `discover` reads back: a top-level
//! `version`, an `object` qualified name, and one `[[claims]]` table per claim
//! with `{ kind, value, column? }`. Nothing else is written — see the export
//! security contract below. The atomic *write* lives in [`super::write`].
//!
//! # Export security contract (spec §2, plan §12 Phase 6)
//!
//! An exported file is meant to be committed to a repository, so it must carry
//! no field that is local, personal, or machine-identifying:
//! - **No opaque profile identity.** The object is written as its qualified
//!   `catalog.schema.object` name; the `ProfileIdentity` the claim is bound to
//!   never reaches the bytes.
//! - **No evidence references.** Session ids and turn ordinals are local and
//!   personal; they are not in the file shape and are not emitted.
//! - **No conversations, SQL, result values, or credentials.** Only the claim's
//!   own value text is written, and that text already passed the payload
//!   constructors' validation (no control characters, no oversized values).
//! - **No absolute paths and no machine/user names.** The file body carries no
//!   path at all; the destination filename is the caller's concern.

use saya_store::StoredClaim;
use saya_types::{ClaimPayload, DatabaseObjectRef};

/// One object's confirmed claims ready to be written to one TOML file.
#[derive(Debug, Clone)]
pub(crate) struct ExportObject {
    pub object: DatabaseObjectRef,
    pub claims: Vec<StoredClaim>,
}

/// Render one object's confirmed claims to the discovered-shape TOML. Claims
/// whose payload cannot be represented in the v1 discovered shape (a
/// `Relationship` claim has no kind word in the shared parser) are skipped — the
/// format does not carry them, and skipping is reported by the caller, not
/// silently lost in the bytes. Returns `None` when no claim survives.
pub(crate) fn render(object: &ExportObject) -> Option<String> {
    let mut entries = Vec::new();
    for claim in &object.claims {
        let Some(payload) = claim.payload.as_ref() else {
            continue;
        };
        if let Some(entry) = render_entry(payload) {
            entries.push(entry);
        }
    }
    if entries.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str("version = 1\n");
    out.push_str(&format!(
        "object = \"{}\"\n",
        object.object.qualified_name()
    ));
    for entry in entries {
        out.push_str("[[claims]]\n");
        out.push_str(&entry);
    }
    Some(out)
}

/// One claim as `kind = "…"\nvalue = "…"\n[column = "…"\n]`. Returns `None` for
/// a payload the v1 discovered shape cannot represent.
///
/// `default_time_column` is written as `{ kind = "time-column", value = <column> }`
/// with **no** `column` field: the shared `build_payload` rejects a `column` for
/// the time-column kind, so emitting one would break the round-trip. The column
/// name is the value, exactly as 6a's discovery reads it back.
fn render_entry(payload: &ClaimPayload) -> Option<String> {
    let (kind, value, column) = match payload {
        ClaimPayload::TableDescription { text, .. } => ("description", text.clone(), None),
        ClaimPayload::TableAlias { alias, .. } => ("alias", alias.clone(), None),
        ClaimPayload::TableGrain { description, .. } => ("grain", description.clone(), None),
        ClaimPayload::ColumnDescription { column, text, .. } => {
            ("column-description", text.clone(), Some(column.clone()))
        }
        ClaimPayload::ColumnRole { column, role, .. } => (
            "column-role",
            role.as_str().to_string(),
            Some(column.clone()),
        ),
        ClaimPayload::DefaultTimeColumn { column, .. } => ("time-column", column.clone(), None),
        // Relationship has no kind word in the shared parser (ClaimKindArg), so
        // it cannot be written to or read from the v1 discovered shape.
        _ => return None,
    };
    let mut out = format!("kind = \"{}\"\nvalue = {}\n", kind, toml_value(&value));
    if let Some(column) = column {
        out.push_str(&format!("column = {}\n", toml_value(&column)));
    }
    Some(out)
}

/// Quote a string as a TOML basic-string literal. The payload constructors
/// reject control characters, so the value is TOML-safe; this escapes the few
/// characters a basic string must escape (`"` and `\`) rather than leaning on a
/// heavier serializer for one line.
fn toml_value(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::{DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity};

    fn object() -> DatabaseObjectRef {
        DatabaseObjectRef::new(
            ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap(),
            "analytics",
            "public",
            "orders",
            DatabaseObjectKind::Table,
        )
        .unwrap()
    }

    /// A `default_time_column` claim is written with the column name as the
    /// value and **no** `column` field. `build_payload` rejects a `column` for
    /// the time-column kind, so emitting one would break the round-trip — this
    /// test pins the shape 6a's discovery reads back.
    #[test]
    fn default_time_column_has_no_column_field() {
        let body = render(&ExportObject {
            object: object(),
            claims: vec![saya_store::StoredClaim {
                id: saya_types::ClaimId::parse("c-x").unwrap(),
                object: object(),
                payload: Some(ClaimPayload::default_time_column("created_at").unwrap()),
                origin: saya_types::ClaimOrigin::UserExplicit,
                status: saya_types::ClaimStatus::Confirmed,
                schema_fingerprint: saya_types::SchemaFingerprint::from_parts(1, &"0".repeat(64))
                    .unwrap(),
                referenced_columns: vec![],
                created_unix_ms: 0,
                updated_unix_ms: 0,
                last_verified_unix_ms: None,
            }],
        })
        .unwrap();
        assert!(body.contains("kind = \"time-column\""));
        assert!(body.contains("value = \"created_at\""));
        assert!(
            !body.contains("column ="),
            "time-column must not emit a column field (breaks round-trip): {body}"
        );
    }

    /// A column-role claim carries the role string as the value and the column
    /// name in the `column` field — the shape `build_payload` for `column-role`
    /// reads back.
    #[test]
    fn column_role_carries_role_and_column() {
        let body = render(&ExportObject {
            object: object(),
            claims: vec![saya_store::StoredClaim {
                id: saya_types::ClaimId::parse("c-x").unwrap(),
                object: object(),
                payload: Some(
                    ClaimPayload::column_role("amount", saya_types::ColumnRole::Measure).unwrap(),
                ),
                origin: saya_types::ClaimOrigin::UserExplicit,
                status: saya_types::ClaimStatus::Confirmed,
                schema_fingerprint: saya_types::SchemaFingerprint::from_parts(1, &"0".repeat(64))
                    .unwrap(),
                referenced_columns: vec![],
                created_unix_ms: 0,
                updated_unix_ms: 0,
                last_verified_unix_ms: None,
            }],
        })
        .unwrap();
        assert!(body.contains("kind = \"column-role\""));
        assert!(body.contains("value = \"measure\""));
        assert!(body.contains("column = \"amount\""));
    }
}
