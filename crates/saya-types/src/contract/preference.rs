//! Scoped user preferences (Plan §12 Phase 5).
//!
//! A preference is **not a claim**. A claim is about a database object, bound to
//! a qualified name and a schema fingerprint, and stale when that object drifts.
//! "I work in Europe/London" is about a *person* — no object, no fingerprint, no
//! drift — so it gets its own typed value, not a `ClaimPayload` variant.
//!
//! Every variant below is a closed enum or a bounded, shape-validated string.
//! There is deliberately no free-text variant: a preference must never carry SQL,
//! secrets, or free-form instructions, and the way to guarantee that is to make
//! it *unrepresentable* rather than to filter it.

use serde::{Deserialize, Serialize};

use crate::contract::error::ContractError;
use crate::contract::identity::ProfileIdentity;
use crate::contract::scope::ScopeRequirement;

/// The maximum length of a timezone string. IANA names are short; 64 is far
/// above the longest real one and keeps the column cheap.
pub const MAX_TIMEZONE_CHARS: usize = 64;
/// The maximum length of a profile *name*. Names come from `connections.toml`,
/// not the identity; a sensible bound keeps the column cheap.
pub const MAX_PROFILE_NAME_CHARS: usize = 128;

/// A bounded user preference. Closed enums and shape-validated strings only;
/// no free-text variant exists by design.
///
/// `Deserialize` is hand-rolled, not derived: the string-carrying variants run
/// their shape validators on deserialization too, so a `{"kind":"timezone",
/// "value":"SELECT..."}` row is refused by the *type*, not only by the store's
/// admission gate. A derived `Deserialize` would populate the field directly
/// and bypass the constructors — exactly the "validated constructor beside a
/// publicly-constructible variant" the security standard warns about, and the
/// spec's "make it unrepresentable, not filtered" rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PreferenceValue {
    #[non_exhaustive]
    Timezone { value: String },
    #[non_exhaustive]
    DateGrain { grain: DateGrain },
    #[non_exhaustive]
    OutputStyle { style: OutputStyle },
    #[non_exhaustive]
    DefaultProfile { name: String },
}

/// The reporting grain for date-shaped results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DateGrain {
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

/// How a result is rendered in the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OutputStyle {
    Table,
    Compact,
    Narrative,
}

/// What a preference applies to. Presentation choices are global; database-shaped
/// choices are scoped to one connection profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PreferenceScope {
    Global,
    Profile(ProfileIdentity),
}

impl PreferenceValue {
    /// The stable discriminator persisted as `preference_kind`, and used as
    /// part of the store's primary key.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Timezone { .. } => "timezone",
            Self::DateGrain { .. } => "date_grain",
            Self::OutputStyle { .. } => "output_style",
            Self::DefaultProfile { .. } => "default_profile",
        }
    }

    /// The scope this value must be stored under. A value set at the wrong
    /// scope is a typed error, never a silent coercion.
    pub fn required_scope(&self) -> ScopeRequirement {
        match self {
            Self::Timezone { .. } | Self::DateGrain { .. } => ScopeRequirement::Profile,
            Self::OutputStyle { .. } | Self::DefaultProfile { .. } => ScopeRequirement::Global,
        }
    }

    /// True if `scope` is the one this value requires. Convenience over
    /// `required_scope().matches(scope)`.
    pub fn matches_scope(&self, scope: &PreferenceScope) -> bool {
        self.required_scope().matches(scope)
    }

    /// Construct a timezone preference. Validation is *shape only*: non-empty,
    /// ≤64 chars, ASCII alphanumeric plus `/`, `_`, `+` and `-`. A wrong-but-
    /// well-shaped timezone is a user error they can see and fix; bundling an
    /// IANA list to validate membership would be a maintenance burden that goes
    /// stale. A fictional-but-well-shaped name is accepted on purpose.
    pub fn timezone(value: impl AsRef<str>) -> Result<Self, ContractError> {
        let value = value.as_ref();
        validate_timezone(value)?;
        Ok(Self::Timezone {
            value: value.to_owned(),
        })
    }

    pub fn date_grain(grain: DateGrain) -> Self {
        Self::DateGrain { grain }
    }

    pub fn output_style(style: OutputStyle) -> Self {
        Self::OutputStyle { style }
    }

    /// Construct a default-profile preference from a profile *name*. The name is
    /// validated by the same shape rules as a database object name: non-empty,
    /// bounded, no control characters. A name is not an identity — it never
    /// reaches the database bytes as `p-…`, and it is resolved against the live
    /// `connections.toml` at use time, not stored as a pointer to a profile.
    pub fn default_profile(name: impl AsRef<str>) -> Result<Self, ContractError> {
        let name = name.as_ref();
        validate_profile_name(name)?;
        Ok(Self::DefaultProfile {
            name: name.to_owned(),
        })
    }

    /// The persisted string for a `Timezone` value, or `None` for other kinds.
    /// Used only by tests that assert a round-tripped value byte-for-byte.
    pub fn timezone_value(&self) -> Option<&str> {
        match self {
            Self::Timezone { value } => Some(value),
            _ => None,
        }
    }

    /// The persisted name for a `DefaultProfile` value, or `None` for other kinds.
    pub fn default_profile_name(&self) -> Option<&str> {
        match self {
            Self::DefaultProfile { name } => Some(name),
            _ => None,
        }
    }
}

/// Validates a timezone by shape: non-empty, ≤`MAX_TIMEZONE_CHARS` chars, ASCII
/// alphanumeric plus `/`, `_`, `+` and `-`. Shape, not membership.
fn validate_timezone(value: &str) -> Result<(), ContractError> {
    if value.is_empty() {
        return Err(ContractError::InvalidTimezone);
    }
    if value.len() > MAX_TIMEZONE_CHARS {
        return Err(ContractError::InvalidTimezone);
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'+' | b'-'))
    {
        return Err(ContractError::InvalidTimezone);
    }
    Ok(())
}

/// Validates a profile name by shape: non-empty, ≤`MAX_PROFILE_NAME_CHARS`
/// chars, no control characters. Reuses the same notion of "name" as
/// `validate_name` in `identity` (which database object names go through),
/// without the identity's hex profile prefix.
fn validate_profile_name(value: &str) -> Result<(), ContractError> {
    if value.is_empty() {
        return Err(ContractError::InvalidProfileName);
    }
    if value.chars().count() > MAX_PROFILE_NAME_CHARS {
        return Err(ContractError::InvalidProfileName);
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(ContractError::InvalidProfileName);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timezone_accepts_well_shaped_real_name() {
        let v = PreferenceValue::timezone("Europe/London").unwrap();
        assert_eq!(v.kind(), "timezone");
        assert_eq!(v.timezone_value(), Some("Europe/London"));
        assert_eq!(v.required_scope(), ScopeRequirement::Profile);
    }

    #[test]
    fn timezone_accepts_well_shaped_fictional_name() {
        // Shape only: a fictional-but-well-shaped name is a user error they can
        // see and fix, not a rejection at storage. Bundling an IANA list is out.
        assert!(PreferenceValue::timezone("Mars/Olympus_Mons").is_ok());
        assert!(PreferenceValue::timezone("Etc/GMT+5").is_ok());
    }

    #[test]
    fn timezone_rejects_malformed_shapes() {
        assert!(PreferenceValue::timezone("").is_err());
        assert!(PreferenceValue::timezone("Europe/London!").is_err());
        assert!(PreferenceValue::timezone("has space").is_err());
        assert!(PreferenceValue::timezone("Europe\\London").is_err());
        assert!(PreferenceValue::timezone("Europe\nLondon").is_err());
        assert!(PreferenceValue::timezone("x".repeat(MAX_TIMEZONE_CHARS + 1)).is_err());
    }

    #[test]
    fn date_grain_round_trips_and_is_profile_scoped() {
        for grain in [
            DateGrain::Day,
            DateGrain::Week,
            DateGrain::Month,
            DateGrain::Quarter,
            DateGrain::Year,
        ] {
            let v = PreferenceValue::date_grain(grain);
            assert_eq!(v.kind(), "date_grain");
            assert_eq!(v.required_scope(), ScopeRequirement::Profile);
            let json = serde_json::to_string(&v).unwrap();
            let back: PreferenceValue = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn output_style_round_trips_and_is_global_scoped() {
        for style in [
            OutputStyle::Table,
            OutputStyle::Compact,
            OutputStyle::Narrative,
        ] {
            let v = PreferenceValue::output_style(style);
            assert_eq!(v.kind(), "output_style");
            assert_eq!(v.required_scope(), ScopeRequirement::Global);
            let json = serde_json::to_string(&v).unwrap();
            let back: PreferenceValue = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn default_profile_accepts_name_and_is_global_scoped() {
        let v = PreferenceValue::default_profile("warehouse").unwrap();
        assert_eq!(v.kind(), "default_profile");
        assert_eq!(v.default_profile_name(), Some("warehouse"));
        assert_eq!(v.required_scope(), ScopeRequirement::Global);
    }

    #[test]
    fn default_profile_rejects_bad_names() {
        // Profile names come from `connections.toml`, where a name is an
        // arbitrary map key — config does not reject spaces. A name is rejected
        // only for being empty, too long, or carrying control characters: the
        // same shape `validate_name` applies to database object names. A space
        // is a legitimate character in a profile name, so it is accepted.
        assert!(PreferenceValue::default_profile("").is_err());
        assert!(PreferenceValue::default_profile("name\n").is_err());
        assert!(PreferenceValue::default_profile("name\u{0}").is_err());
        assert!(PreferenceValue::default_profile("x".repeat(MAX_PROFILE_NAME_CHARS + 1)).is_err());
        assert!(PreferenceValue::default_profile("has space").is_ok());
    }

    #[test]
    fn timezone_round_trips_through_serde() {
        let v = PreferenceValue::timezone("America/New_York").unwrap();
        let json = serde_json::to_string(&v).unwrap();
        let back: PreferenceValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}
