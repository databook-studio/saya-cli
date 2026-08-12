use serde::{Deserialize, Serialize};

use crate::contract::claim::{MAX_REFERENCED_COLUMNS, validate_text};
use crate::contract::claim_enums::{Cardinality, ColumnRole};
use crate::contract::error::ContractError;
use crate::contract::identity::{DatabaseObjectRef, validate_name};

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
    use crate::contract::claim::MAX_TEXT_CHARS;
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
}
