use serde::{Deserialize, Serialize};

use crate::contract::error::ContractError;

pub const MAX_NAME_CHARS: usize = 128;

/// Opaque, validated profile identity: the literal prefix `p-` followed by 64 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ProfileIdentity(String);

impl ProfileIdentity {
    pub fn parse(value: &str) -> Result<Self, ContractError> {
        if value.len() != 66 || !value.starts_with("p-") {
            return Err(ContractError::InvalidProfileIdentity);
        }
        if !value[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ContractError::InvalidProfileIdentity);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProfileIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for ProfileIdentity {
    type Error = ContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseObjectKind {
    Table,
    View,
}

impl DatabaseObjectKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::View => "view",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "table" => Some(Self::Table),
            "view" => Some(Self::View),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DatabaseObjectRef {
    profile: ProfileIdentity,
    catalog: String,
    schema: String,
    object: String,
    kind: DatabaseObjectKind,
}

impl DatabaseObjectRef {
    pub fn new(
        profile: ProfileIdentity,
        catalog: impl Into<String>,
        schema: impl Into<String>,
        object: impl Into<String>,
        kind: DatabaseObjectKind,
    ) -> Result<Self, ContractError> {
        let catalog = catalog.into();
        let schema = schema.into();
        let object = object.into();
        validate_name(&catalog)?;
        validate_name(&schema)?;
        validate_name(&object)?;
        Ok(Self {
            profile,
            catalog,
            schema,
            object,
            kind,
        })
    }

    pub fn profile(&self) -> &ProfileIdentity {
        &self.profile
    }

    pub fn catalog(&self) -> &str {
        &self.catalog
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    pub fn object(&self) -> &str {
        &self.object
    }

    pub fn kind(&self) -> DatabaseObjectKind {
        self.kind
    }

    pub fn qualified_name(&self) -> String {
        format!("{}.{}.{}", self.catalog, self.schema, self.object)
    }
}

pub(crate) fn validate_name(value: &str) -> Result<(), ContractError> {
    if value.is_empty() {
        return Err(ContractError::EmptyName);
    }
    if value.chars().count() > MAX_NAME_CHARS {
        return Err(ContractError::NameTooLong);
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(ContractError::ControlCharacter);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_profile_identity() {
        let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let input = format!("p-{hex}");
        let id = ProfileIdentity::parse(&input).unwrap();
        assert_eq!(id.as_str(), &input);
    }

    #[test]
    fn parse_rejects_too_short() {
        assert!(ProfileIdentity::parse("p-abc").is_err());
    }

    #[test]
    fn parse_rejects_too_long() {
        let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789ff";
        assert!(ProfileIdentity::parse(&format!("p-{hex}")).is_err());
    }

    #[test]
    fn parse_rejects_missing_prefix() {
        let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        assert!(ProfileIdentity::parse(hex).is_err());
    }

    #[test]
    fn parse_rejects_uppercase_hex() {
        let hex = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";
        assert!(ProfileIdentity::parse(&format!("p-{hex}")).is_err());
    }

    #[test]
    fn parse_rejects_non_hex() {
        let hex = "gggggg0123456789gggggg0123456789gggggg0123456789gggggg0123456789";
        assert!(ProfileIdentity::parse(&format!("p-{hex}")).is_err());
    }

    #[test]
    fn display_delegates_to_as_str() {
        let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let input = format!("p-{hex}");
        let id = ProfileIdentity::parse(&input).unwrap();
        assert_eq!(format!("{id}"), input);
    }

    #[test]
    fn serde_round_trip() {
        let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let input = format!("p-{hex}");
        let id = ProfileIdentity::parse(&input).unwrap();
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: ProfileIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn serde_rejects_invalid_string() {
        let result: Result<ProfileIdentity, _> = serde_json::from_str(r#""p-INVALID""#);
        assert!(result.is_err());
    }

    #[test]
    fn database_object_kind_as_str() {
        assert_eq!(DatabaseObjectKind::Table.as_str(), "table");
        assert_eq!(DatabaseObjectKind::View.as_str(), "view");
    }

    #[test]
    fn database_object_kind_parse() {
        assert_eq!(
            DatabaseObjectKind::parse("table"),
            Some(DatabaseObjectKind::Table)
        );
        assert_eq!(
            DatabaseObjectKind::parse("view"),
            Some(DatabaseObjectKind::View)
        );
        assert_eq!(DatabaseObjectKind::parse("unknown"), None);
    }

    #[test]
    fn database_object_ref_new_rejects_empty() {
        let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
        let kind = DatabaseObjectKind::Table;
        assert!(DatabaseObjectRef::new(profile.clone(), "", "sch", "obj", kind).is_err());
        assert!(DatabaseObjectRef::new(profile.clone(), "cat", "", "obj", kind).is_err());
        assert!(DatabaseObjectRef::new(profile, "cat", "sch", "", kind).is_err());
    }

    #[test]
    fn database_object_ref_new_rejects_too_long() {
        let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
        let long = "a".repeat(129);
        let kind = DatabaseObjectKind::Table;
        assert!(DatabaseObjectRef::new(profile.clone(), &long, "sch", "obj", kind).is_err());
        assert!(DatabaseObjectRef::new(profile.clone(), "cat", &long, "obj", kind).is_err());
        assert!(DatabaseObjectRef::new(profile, "cat", "sch", &long, kind).is_err());
    }

    #[test]
    fn database_object_ref_new_rejects_control_chars() {
        let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
        let kind = DatabaseObjectKind::Table;
        assert!(DatabaseObjectRef::new(profile.clone(), "cat\n", "sch", "obj", kind).is_err());
        assert!(DatabaseObjectRef::new(profile.clone(), "cat", "sch\u{0}", "obj", kind).is_err());
        assert!(DatabaseObjectRef::new(profile, "cat", "sch", "obj\r", kind).is_err());
    }

    #[test]
    fn database_object_ref_getters() {
        let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
        let r = DatabaseObjectRef::new(
            profile.clone(),
            "cat",
            "sch",
            "obj",
            DatabaseObjectKind::View,
        )
        .unwrap();
        assert_eq!(r.profile(), &profile);
        assert_eq!(r.catalog(), "cat");
        assert_eq!(r.schema(), "sch");
        assert_eq!(r.object(), "obj");
        assert_eq!(r.kind(), DatabaseObjectKind::View);
    }

    #[test]
    fn qualified_name_format() {
        let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
        let r = DatabaseObjectRef::new(profile, "cat", "sch", "obj", DatabaseObjectKind::Table)
            .unwrap();
        assert_eq!(r.qualified_name(), "cat.sch.obj");
    }

    #[test]
    fn cross_profile_isolation() {
        let hex_a = format!("p-{}", "a".repeat(64));
        let hex_b = format!("p-{}", "b".repeat(64));
        let profile_a = ProfileIdentity::parse(&hex_a).unwrap();
        let profile_b = ProfileIdentity::parse(&hex_b).unwrap();
        let ref_a =
            DatabaseObjectRef::new(profile_a, "cat", "sch", "obj", DatabaseObjectKind::Table)
                .unwrap();
        let ref_b =
            DatabaseObjectRef::new(profile_b, "cat", "sch", "obj", DatabaseObjectKind::Table)
                .unwrap();
        assert_ne!(ref_a, ref_b);
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(ref_a.clone());
        assert!(!set.contains(&ref_b));
    }
}
