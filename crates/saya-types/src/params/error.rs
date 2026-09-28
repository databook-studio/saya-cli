//! Why a parameter declaration or a bound value is refused.

use thiserror::Error;

use super::spec::{MAX_PARAM_DESCRIPTION_BYTES, MAX_PARAMETERS};

/// Every way a parameter contract can be violated: the declared-spec checks
/// (name, description, list bounds) and the strict value parses. Messages
/// name the bound at fault; they never echo a bound value — only names,
/// which already appear in the SQL.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ParamError {
    #[error("parameter name {0:?} must match [a-z_][a-z0-9_]{{0,31}}")]
    InvalidName(String),
    #[error("at most {MAX_PARAMETERS} parameters may be declared ({0} given)")]
    TooManySpecs(usize),
    #[error("parameter {0:?} is declared more than once")]
    DuplicateName(String),
    #[error("parameter description is over the {MAX_PARAM_DESCRIPTION_BYTES}-byte limit")]
    DescriptionTooLong,
    #[error("parameter description contains control characters")]
    DescriptionControlCharacter,
    #[error("value is not a valid integer: {0}")]
    NotAnInteger(&'static str),
    #[error("value is not a valid boolean: {0}")]
    NotABoolean(&'static str),
    #[error("value is not a valid decimal: {0}")]
    NotADecimal(&'static str),
    #[error("value is not a valid date: {0}")]
    NotADate(&'static str),
    #[error("value is not a valid timestamp: {0}")]
    NotATimestamp(&'static str),
}
