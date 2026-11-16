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
mod tests;

#[cfg(test)]
mod property_tests;
