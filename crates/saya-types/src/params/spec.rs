//! Declared parameter specifications and bound parameters: the contract
//! between a saved investigation's SQL and the values a run supplies.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::error::ParamError;
use super::value::{ParamType, ParamValue};

/// Cap on declared parameters per definition.
pub const MAX_PARAMETERS: usize = 32;
/// A parameter name is `[a-z_][a-z0-9_]{0,31}`: at most 32 ASCII characters.
pub const MAX_PARAM_NAME_CHARS: usize = 32;
/// Cap on one parameter description, in bytes.
pub const MAX_PARAM_DESCRIPTION_BYTES: usize = 256;

/// Whether `name` matches `[a-z_][a-z0-9_]{0,31}` — the only shape a
/// parameter name may take, in declarations and bindings alike.
pub fn is_valid_param_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some((first, rest)) = bytes.split_first() else {
        return false;
    };
    let head = first.is_ascii_lowercase() || *first == b'_';
    head && rest.len() < MAX_PARAM_NAME_CHARS
        && rest
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

/// One declared parameter of a saved investigation. The portable document
/// spells the type field `type`, which is not a Rust identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: ParamType,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ParameterSpec {
    /// Builds and validates one specification.
    pub fn new(
        name: impl Into<String>,
        param_type: ParamType,
        required: bool,
        description: Option<String>,
    ) -> Result<Self, ParamError> {
        let spec = Self {
            name: name.into(),
            param_type,
            required,
            description,
        };
        spec.validate()?;
        Ok(spec)
    }

    /// Re-checks the per-spec bounds: the name shape, and the description's
    /// byte cap and control-character refusal. Fields are public, so JSON
    /// and code both bypass `new`; every gate re-runs these checks.
    pub fn validate(&self) -> Result<(), ParamError> {
        if !is_valid_param_name(&self.name) {
            return Err(ParamError::InvalidName(self.name.clone()));
        }
        if let Some(description) = &self.description {
            if description.len() > MAX_PARAM_DESCRIPTION_BYTES {
                return Err(ParamError::DescriptionTooLong);
            }
            if description.chars().any(char::is_control) {
                return Err(ParamError::DescriptionControlCharacter);
            }
        }
        Ok(())
    }

    /// Validates a declared list: the count cap, every per-spec bound, and
    /// unique names. Saved-investigation validation and the binding path
    /// both call this.
    pub fn validate_list(specs: &[Self]) -> Result<(), ParamError> {
        if specs.len() > MAX_PARAMETERS {
            return Err(ParamError::TooManySpecs(specs.len()));
        }
        let mut seen = HashSet::new();
        for spec in specs {
            spec.validate()?;
            if !seen.insert(&spec.name) {
                return Err(ParamError::DuplicateName(spec.name.clone()));
            }
        }
        Ok(())
    }
}

/// One bound value for a named parameter, as carried by a
/// [`crate::query::QueryRequest`]. Values live only in the request: they
/// never persist in definitions, bindings, audit records, or evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundParam {
    pub name: String,
    pub value: ParamValue,
}
