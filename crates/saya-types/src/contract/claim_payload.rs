use serde::{Deserialize, Serialize};

use crate::contract::claim::{MAX_REFERENCED_COLUMNS, validate_text};
use crate::contract::claim_enums::{Cardinality, ColumnRole};
use crate::contract::error::ContractError;
use crate::contract::identity::{DatabaseObjectRef, validate_name};
use crate::schema::Table;

/// A referenced column snapshotted at claim time: its name plus the type and
/// nullability the connector reported when the claim was made.
///
/// Phase 5a persists this so a later schema change can tell a *retyped*
/// referenced column (the claim may now be wrong) from an *unrelated* column
/// changing elsewhere (the claim is fine). A claim proposed without a live
/// table stores no snapshot; `data_type` is then empty and `nullable` is
/// `false`, which the reconciler treats as "unknown" rather than "matched".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferencedColumn {
    pub name: String,
    /// The column's type as the connector reported it when the claim was made.
    /// Empty for old `["a","b"]`-shape rows upgraded in place by the store.
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
// Every variant is `#[non_exhaustive]` as well as the enum. Without it another
// crate could write `ClaimPayload::TableDescription { text }` directly and skip
// the validation below, which is the only thing keeping oversized and
// control-character-bearing text out of a rendered context block.
pub enum ClaimPayload {
    #[non_exhaustive]
    TableDescription { text: String },
    #[non_exhaustive]
    TableAlias { alias: String },
    #[non_exhaustive]
    TableGrain { description: String },
    #[non_exhaustive]
    ColumnDescription { column: String, text: String },
    #[non_exhaustive]
    ColumnRole { column: String, role: ColumnRole },
    #[non_exhaustive]
    DefaultTimeColumn { column: String },
    #[non_exhaustive]
    Relationship {
        target: DatabaseObjectRef,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        cardinality: Cardinality,
    },
}

impl ClaimPayload {
    /// Stable discriminator used as part of a claim's deduplication key.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TableDescription { .. } => "table_description",
            Self::TableAlias { .. } => "table_alias",
            Self::TableGrain { .. } => "table_grain",
            Self::ColumnDescription { .. } => "column_description",
            Self::ColumnRole { .. } => "column_role",
            Self::DefaultTimeColumn { .. } => "default_time_column",
            Self::Relationship { .. } => "relationship",
        }
    }

    /// Columns this claim depends on, for schema-drift reconciliation. Empty for table-level claims.
    pub fn referenced_columns(&self) -> Vec<&str> {
        match self {
            Self::TableDescription { .. } => Vec::new(),
            Self::TableAlias { .. } => Vec::new(),
            Self::TableGrain { .. } => Vec::new(),
            Self::ColumnDescription { column, .. } => vec![column],
            Self::ColumnRole { column, .. } => vec![column],
            Self::DefaultTimeColumn { column } => vec![column],
            Self::Relationship { local_columns, .. } => {
                local_columns.iter().map(|s| s.as_str()).collect()
            }
        }
    }

    /// Snapshots of this claim's referenced columns, resolved against `table`.
    /// A referenced column absent from `table` is skipped — a claim cannot
    /// snapshot what does not exist, and the caller decides what that means.
    /// Name matching is case-insensitive, the same convention the schema
    /// fingerprint and the validity reconciler use, so a column that kept its
    /// name but changed type resolves here and is detected as a change later.
    pub fn referenced_column_snapshots(&self, table: &Table) -> Vec<ReferencedColumn> {
        self.referenced_columns()
            .iter()
            .filter_map(|name| {
                table
                    .columns
                    .iter()
                    .find(|col| col.name.eq_ignore_ascii_case(name))
                    .map(|col| ReferencedColumn {
                        name: name.to_string(),
                        data_type: col.data_type.clone(),
                        nullable: col.nullable,
                    })
            })
            .collect()
    }

    /// Name-only snapshots of this claim's referenced columns, for a caller
    /// that has no live schema to resolve types against. Each referenced
    /// column becomes a snapshot with an empty `data_type` and `nullable:
    /// false`, which the reconciler treats as *unknown* rather than matched —
    /// so a no-schema proposal still records the column names a later drift
    /// rule needs (a removed referenced column reads as Stale), without ever
    /// claiming a type it never observed.
    pub fn referenced_column_name_snapshots(&self) -> Vec<ReferencedColumn> {
        self.referenced_columns()
            .into_iter()
            .map(|name| ReferencedColumn {
                name: name.to_string(),
                data_type: String::new(),
                nullable: false,
            })
            .collect()
    }

    pub fn table_description(text: impl Into<String>) -> Result<Self, ContractError> {
        let text = text.into();
        validate_text(&text)?;
        Ok(Self::TableDescription { text })
    }

    pub fn table_alias(alias: impl Into<String>) -> Result<Self, ContractError> {
        let alias = alias.into();
        validate_name(&alias)?;
        Ok(Self::TableAlias { alias })
    }

    pub fn table_grain(description: impl Into<String>) -> Result<Self, ContractError> {
        let description = description.into();
        validate_text(&description)?;
        Ok(Self::TableGrain { description })
    }

    pub fn column_description(
        column: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let column = column.into();
        let text = text.into();
        validate_name(&column)?;
        validate_text(&text)?;
        Ok(Self::ColumnDescription { column, text })
    }

    pub fn column_role(column: impl Into<String>, role: ColumnRole) -> Result<Self, ContractError> {
        let column = column.into();
        validate_name(&column)?;
        Ok(Self::ColumnRole { column, role })
    }

    pub fn default_time_column(column: impl Into<String>) -> Result<Self, ContractError> {
        let column = column.into();
        validate_name(&column)?;
        Ok(Self::DefaultTimeColumn { column })
    }

    pub fn relationship(
        target: DatabaseObjectRef,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        cardinality: Cardinality,
    ) -> Result<Self, ContractError> {
        if local_columns.is_empty() || target_columns.is_empty() {
            return Err(ContractError::EmptyColumns);
        }
        if local_columns.len() != target_columns.len() {
            return Err(ContractError::ColumnCountMismatch);
        }
        if local_columns.len() > MAX_REFERENCED_COLUMNS {
            return Err(ContractError::TooManyColumns);
        }
        for col in local_columns.iter().chain(target_columns.iter()) {
            validate_name(col)?;
        }
        Ok(Self::Relationship {
            target,
            local_columns,
            target_columns,
            cardinality,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::claim::{CLAIM_PAYLOAD_VERSION, MAX_TEXT_CHARS};
    use crate::contract::claim_enums::{Cardinality, ColumnRole};
    use crate::{DatabaseObjectKind, ProfileIdentity};

    fn make_profile() -> ProfileIdentity {
        ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
    }

    fn make_target() -> DatabaseObjectRef {
        let profile = make_profile();
        DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap()
    }

    #[test]
    fn constructors_reject_empty_text() {
        assert!(ClaimPayload::table_description("").is_err());
        assert!(ClaimPayload::table_grain("").is_err());
        assert!(ClaimPayload::column_description("col", "").is_err());
    }

    #[test]
    fn constructors_reject_long_text() {
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(ClaimPayload::table_description(&long).is_err());
        assert!(ClaimPayload::table_grain(&long).is_err());
        assert!(ClaimPayload::column_description("col", &long).is_err());
    }

    #[test]
    fn constructors_reject_control_chars() {
        assert!(ClaimPayload::table_description("hello\nworld").is_err());
        assert!(ClaimPayload::table_grain("hello\tworld").is_err());
        assert!(ClaimPayload::column_description("col", "text\n").is_err());
    }

    #[test]
    fn table_alias_rejects_empty() {
        assert!(ClaimPayload::table_alias("").is_err());
    }

    #[test]
    fn column_role_validates_column_name() {
        assert!(ClaimPayload::column_role("", ColumnRole::Dimension).is_err());
        assert!(ClaimPayload::column_role("col\n", ColumnRole::Dimension).is_err());
    }

    #[test]
    fn default_time_column_validates_column_name() {
        assert!(ClaimPayload::default_time_column("").is_err());
    }

    #[test]
    fn relationship_rejects_empty_column_lists() {
        let target = make_target();
        assert!(
            ClaimPayload::relationship(
                target.clone(),
                vec![],
                vec!["b".into()],
                Cardinality::OneToOne
            )
            .is_err()
        );
        assert!(
            ClaimPayload::relationship(
                target.clone(),
                vec!["a".into()],
                vec![],
                Cardinality::OneToOne
            )
            .is_err()
        );
        assert!(
            ClaimPayload::relationship(
                target,
                vec!["a".into()],
                vec!["b".into()],
                Cardinality::OneToOne
            )
            .is_ok()
        );
    }

    #[test]
    fn relationship_rejects_mismatched_lengths() {
        let target = make_target();
        let result = ClaimPayload::relationship(
            target,
            vec!["a".into(), "b".into()],
            vec!["x".into()],
            Cardinality::ManyToOne,
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ContractError::ColumnCountMismatch);
    }

    #[test]
    fn relationship_rejects_too_many_columns() {
        let target = make_target();
        let cols: Vec<String> = (0..=MAX_REFERENCED_COLUMNS)
            .map(|i| format!("c{i}"))
            .collect();
        let result = ClaimPayload::relationship(target, cols.clone(), cols, Cardinality::OneToMany);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ContractError::TooManyColumns);
    }

    #[test]
    fn relationship_validates_column_names() {
        let target = make_target();
        assert!(
            ClaimPayload::relationship(
                target.clone(),
                vec!["".into()],
                vec!["x".into()],
                Cardinality::OneToOne,
            )
            .is_err()
        );
        assert!(
            ClaimPayload::relationship(
                target,
                vec!["x".into()],
                vec!["".into()],
                Cardinality::OneToOne,
            )
            .is_err()
        );
    }

    #[test]
    fn kind_discriminator() {
        let target = make_target();
        assert_eq!(
            ClaimPayload::table_description("desc").unwrap().kind(),
            "table_description"
        );
        assert_eq!(
            ClaimPayload::table_alias("t").unwrap().kind(),
            "table_alias"
        );
        assert_eq!(
            ClaimPayload::table_grain("g").unwrap().kind(),
            "table_grain"
        );
        assert_eq!(
            ClaimPayload::column_description("c", "d").unwrap().kind(),
            "column_description"
        );
        assert_eq!(
            ClaimPayload::column_role("c", ColumnRole::Identifier)
                .unwrap()
                .kind(),
            "column_role"
        );
        assert_eq!(
            ClaimPayload::default_time_column("c").unwrap().kind(),
            "default_time_column"
        );
        assert_eq!(
            ClaimPayload::relationship(
                target,
                vec!["a".into()],
                vec!["b".into()],
                Cardinality::OneToMany
            )
            .unwrap()
            .kind(),
            "relationship"
        );
    }

    #[test]
    fn referenced_columns() {
        let target = make_target();
        assert!(
            ClaimPayload::table_description("desc")
                .unwrap()
                .referenced_columns()
                .is_empty()
        );
        assert!(
            ClaimPayload::table_alias("t")
                .unwrap()
                .referenced_columns()
                .is_empty()
        );
        assert!(
            ClaimPayload::table_grain("g")
                .unwrap()
                .referenced_columns()
                .is_empty()
        );
        assert_eq!(
            ClaimPayload::column_description("c", "d")
                .unwrap()
                .referenced_columns(),
            vec!["c"]
        );
        assert_eq!(
            ClaimPayload::column_role("c", ColumnRole::Measure)
                .unwrap()
                .referenced_columns(),
            vec!["c"]
        );
        assert_eq!(
            ClaimPayload::default_time_column("c")
                .unwrap()
                .referenced_columns(),
            vec!["c"]
        );
        assert_eq!(
            ClaimPayload::relationship(
                target,
                vec!["local_a".into(), "local_b".into()],
                vec!["tgt_a".into(), "tgt_b".into()],
                Cardinality::ManyToMany,
            )
            .unwrap()
            .referenced_columns(),
            vec!["local_a", "local_b"]
        );
    }

    #[test]
    fn serde_round_trip() {
        let payload = ClaimPayload::table_description("hello world").unwrap();
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: ClaimPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload, deserialized);
    }

    fn table_with(cols: &[(&str, &str, bool)]) -> crate::Table {
        crate::Table {
            name: "t".into(),
            columns: cols
                .iter()
                .map(|(name, ty, nullable)| crate::Column {
                    name: (*name).into(),
                    data_type: (*ty).into(),
                    nullable: *nullable,
                })
                .collect(),
        }
    }

    #[test]
    fn snapshots_resolve_type_and_nullability_from_the_live_table() {
        let table = table_with(&[("user_id", "bigint", false), ("amount", "numeric", true)]);
        let payload = ClaimPayload::column_description("user_id", "the user id").unwrap();
        assert_eq!(
            payload.referenced_column_snapshots(&table),
            vec![ReferencedColumn {
                name: "user_id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }]
        );
    }

    #[test]
    fn snapshot_skips_a_referenced_column_absent_from_the_table() {
        // A claim cannot snapshot what does not exist; the caller decides what
        // that means. The helper returns only the columns it could resolve.
        let table = table_with(&[("id", "bigint", false)]);
        let payload = ClaimPayload::column_description("missing", "gone").unwrap();
        assert!(payload.referenced_column_snapshots(&table).is_empty());
    }

    #[test]
    fn snapshots_match_case_insensitively_like_fingerprinting_does() {
        // Schema discovery and fingerprint comparison are case-insensitive on
        // object and column names; a snapshot must resolve the same way or a
        // retyped column would read as "absent" instead of "changed".
        let table = table_with(&[("User_Id", "bigint", false)]);
        let payload = ClaimPayload::column_description("user_id", "the id").unwrap();
        assert_eq!(
            payload.referenced_column_snapshots(&table),
            vec![ReferencedColumn {
                name: "user_id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }]
        );
    }

    #[test]
    fn relationship_snapshots_all_local_columns() {
        let table = table_with(&[
            ("local_a", "int", false),
            ("local_b", "text", true),
            ("unrelated", "int", false),
        ]);
        let target = make_target();
        let payload = ClaimPayload::relationship(
            target,
            vec!["local_a".into(), "local_b".into()],
            vec!["x".into(), "y".into()],
            Cardinality::OneToMany,
        )
        .unwrap();
        assert_eq!(
            payload.referenced_column_snapshots(&table),
            vec![
                ReferencedColumn {
                    name: "local_a".into(),
                    data_type: "int".into(),
                    nullable: false,
                },
                ReferencedColumn {
                    name: "local_b".into(),
                    data_type: "text".into(),
                    nullable: true,
                },
            ]
        );
    }

    #[test]
    fn table_level_claims_snapshot_nothing() {
        let table = table_with(&[("id", "bigint", false)]);
        assert!(
            ClaimPayload::table_description("desc")
                .unwrap()
                .referenced_column_snapshots(&table)
                .is_empty()
        );
        assert!(
            ClaimPayload::table_alias("a")
                .unwrap()
                .referenced_column_snapshots(&table)
                .is_empty()
        );
    }

    #[test]
    fn name_only_snapshots_record_names_without_claiming_a_type() {
        // A no-schema caller records the column names a drift rule needs, but
        // with an empty type so the reconciler treats them as unknown.
        let payload = ClaimPayload::column_description("user_id", "the id").unwrap();
        assert_eq!(
            payload.referenced_column_name_snapshots(),
            vec![ReferencedColumn {
                name: "user_id".into(),
                data_type: String::new(),
                nullable: false,
            }]
        );
        assert!(
            ClaimPayload::table_description("desc")
                .unwrap()
                .referenced_column_name_snapshots()
                .is_empty()
        );
    }

    #[test]
    fn referenced_column_serde_round_trips() {
        let col = ReferencedColumn {
            name: "amount".into(),
            data_type: "numeric".into(),
            nullable: true,
        };
        let json = serde_json::to_string(&col).unwrap();
        let back: ReferencedColumn = serde_json::from_str(&json).unwrap();
        assert_eq!(col, back);
    }

    #[test]
    fn claim_payload_version_is_two() {
        // Phase 5a persists per-column type/nullability snapshots, so a stored
        // claim records payload_version 2. A claim proposed before this change
        // carries version 1 and must still decode (the store handles both).
        assert_eq!(CLAIM_PAYLOAD_VERSION, 2);
    }
}

#[cfg(test)]
mod property_tests {
    //! Property 5 (spec §1): every `ClaimPayload` constructor either returns a
    //! payload whose text round-trips the input, or a typed error — never a
    //! panic, never a payload containing a control character. Pure. This is the
    //! generalisation of the hand-written constructor tests: the constructors are
    //! the only thing keeping oversized and control-character-bearing text out of
    //! a rendered context block, and a property asks whether that holds across an
    //! input space the hand-picked cases did not imagine.
    use super::*;
    use crate::contract::claim_enums::{Cardinality, ColumnRole};
    use crate::{DatabaseObjectKind, ProfileIdentity};
    use proptest::prelude::*;

    fn make_profile() -> ProfileIdentity {
        ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
    }

    fn make_target() -> DatabaseObjectRef {
        let profile = make_profile();
        DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap()
    }

    /// Any text a user might supply as a description/alias/grain/value: empty,
    /// long, control chars, unicode — every rejection path must fire, none panic.
    fn text() -> impl Strategy<Value = String> {
        prop::collection::vec(any::<char>(), 0..=1100).prop_map(|chars| chars.into_iter().collect())
    }

    /// Any column name: same wide alphabet, shorter cap (`MAX_NAME_CHARS`).
    fn col_name() -> impl Strategy<Value = String> {
        prop::collection::vec(any::<char>(), 0..=130).prop_map(|chars| chars.into_iter().collect())
    }

    /// True when `s` contains any Unicode control character.
    fn has_control(s: &str) -> bool {
        s.chars().any(|c| c.is_control())
    }

    /// All five column roles. `select` needs a `Vec`; an array does not satisfy
    /// `Into<Cow<'static, [_]>>`.
    fn role() -> impl Strategy<Value = ColumnRole> {
        prop::sample::select(vec![
            ColumnRole::Identifier,
            ColumnRole::Dimension,
            ColumnRole::Measure,
            ColumnRole::Timestamp,
            ColumnRole::Sensitive,
        ])
    }

    /// Every cardinality.
    fn cardinality() -> impl Strategy<Value = Cardinality> {
        prop::sample::select(vec![
            Cardinality::OneToOne,
            Cardinality::OneToMany,
            Cardinality::ManyToOne,
            Cardinality::ManyToMany,
        ])
    }

    proptest! {
        /// `table_description`: Ok ⇒ stored text round-trips and has no control
        /// char; Err ⇒ typed error; never panics.
        #[test]
        fn table_description_total(s in text()) {
            match ClaimPayload::table_description(&s) {
                Ok(ClaimPayload::TableDescription { text }) => {
                    prop_assert!(!has_control(&text));
                    prop_assert_eq!(&text, &s);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyText | ContractError::TextTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `table_grain`: same totality contract on its `description` field.
        #[test]
        fn table_grain_total(s in text()) {
            match ClaimPayload::table_grain(&s) {
                Ok(ClaimPayload::TableGrain { description }) => {
                    prop_assert!(!has_control(&description));
                    prop_assert_eq!(&description, &s);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyText | ContractError::TextTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `column_description`: Ok ⇒ both `column` and `text` round-trip and hold
        /// no control char; the `column` is a *name* (MAX_NAME_CHARS) and `text`
        /// is a claim body (MAX_TEXT_CHARS), so the two length bounds are separate.
        #[test]
        fn column_description_total(col in col_name(), body in text()) {
            match ClaimPayload::column_description(&col, &body) {
                Ok(ClaimPayload::ColumnDescription { column, text }) => {
                    prop_assert!(!has_control(&column));
                    prop_assert!(!has_control(&text));
                    prop_assert_eq!(&column, &col);
                    prop_assert_eq!(&text, &body);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ControlCharacter
                        | ContractError::EmptyText | ContractError::TextTooLong),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `table_alias`: a name, validated by `validate_name`.
        #[test]
        fn table_alias_total(s in col_name()) {
            match ClaimPayload::table_alias(&s) {
                Ok(ClaimPayload::TableAlias { alias }) => {
                    prop_assert!(!has_control(&alias));
                    prop_assert_eq!(&alias, &s);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `column_role`: a name plus a role; Ok ⇒ name round-trips and has no
        /// control char; the role is echoed back by `referenced_columns`.
        #[test]
        fn column_role_total(col in col_name(), r in role()) {
            match ClaimPayload::column_role(&col, r) {
                Ok(ClaimPayload::ColumnRole { column, role }) => {
                    prop_assert!(!has_control(&column));
                    prop_assert_eq!(&column, &col);
                    prop_assert_eq!(role, r);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `default_time_column`: a name.
        #[test]
        fn default_time_column_total(s in col_name()) {
            match ClaimPayload::default_time_column(&s) {
                Ok(ClaimPayload::DefaultTimeColumn { column }) => {
                    prop_assert!(!has_control(&column));
                    prop_assert_eq!(&column, &s);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `relationship`: the two column lists and the cardinality round-trip,
        /// every stored column is control-char-free, and the count/length errors
        /// are typed. Generates up to a few columns per side.
        #[test]
        fn relationship_total(
            locals in prop::collection::vec(col_name(), 0..=4),
            targets in prop::collection::vec(col_name(), 0..=4),
            card in cardinality(),
        ) {
            let target = make_target();
            match ClaimPayload::relationship(target, locals.clone(), targets.clone(), card) {
                Ok(ClaimPayload::Relationship {
                    local_columns,
                    target_columns,
                    cardinality,
                    ..
                }) => {
                    for c in local_columns.iter().chain(target_columns.iter()) {
                        prop_assert!(!has_control(c), "control char in relationship column: {c:?}");
                    }
                    prop_assert_eq!(&local_columns, &locals);
                    prop_assert_eq!(&target_columns, &targets);
                    prop_assert_eq!(cardinality, card);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyColumns | ContractError::ColumnCountMismatch
                        | ContractError::TooManyColumns
                        | ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }
    }
}
