//! The declared parameter type and the validated value bound to it.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{error::ParamError, parse};

/// The declared type of an investigation parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ParamType {
    String,
    Integer,
    Boolean,
    Decimal,
    Date,
    Timestamp,
}

impl ParamType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Decimal => "decimal",
            Self::Date => "date",
            Self::Timestamp => "timestamp",
        }
    }
}

impl fmt::Display for ParamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A typed runtime value for one named parameter. `Decimal`, `Date`, and
/// `Timestamp` keep their validated text exactly as bound.
///
/// Values are sensitive: the [`Debug`] impl prints the variant only, never
/// the value, so bindings survive logs and transcripts without leaking.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamValue {
    Null,
    String(String),
    Integer(i64),
    Boolean(bool),
    Decimal(String),
    Date(String),
    Timestamp(String),
}

impl fmt::Debug for ParamValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Null => "Null",
            Self::String(_) => "String(_)",
            Self::Integer(_) => "Integer(_)",
            Self::Boolean(_) => "Boolean(_)",
            Self::Decimal(_) => "Decimal(_)",
            Self::Date(_) => "Date(_)",
            Self::Timestamp(_) => "Timestamp(_)",
        })
    }
}

impl ParamValue {
    /// Parses `raw` (for example from `--param name=value`) as a value of
    /// the declared type. The literal `null` is the explicit null for any
    /// type — which also means a string value `null` is not expressible
    /// here. Parsing is strict: no trimming, no coercion, no `+` sign, no
    /// exponent, no bare local time.
    pub fn parse(param_type: ParamType, raw: &str) -> Result<Self, ParamError> {
        if raw == "null" {
            return Ok(Self::Null);
        }
        match param_type {
            ParamType::String => Ok(Self::String(raw.to_owned())),
            ParamType::Integer => parse::integer(raw)
                .map(Self::Integer)
                .map_err(ParamError::NotAnInteger),
            ParamType::Boolean => parse::boolean(raw)
                .map(Self::Boolean)
                .map_err(ParamError::NotABoolean),
            ParamType::Decimal => parse::decimal(raw)
                .map(|_| Self::Decimal(raw.to_owned()))
                .map_err(ParamError::NotADecimal),
            ParamType::Date => parse::date(raw)
                .map(|_| Self::Date(raw.to_owned()))
                .map_err(ParamError::NotADate),
            ParamType::Timestamp => parse::timestamp(raw)
                .map(|_| Self::Timestamp(raw.to_owned()))
                .map_err(ParamError::NotATimestamp),
        }
    }
}
