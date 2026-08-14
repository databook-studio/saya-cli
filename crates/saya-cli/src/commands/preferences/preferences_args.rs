//! Parsing and validation for `saya preferences` arguments. Presentation only:
//! this module builds the typed value and maps every fallible `saya-types`
//! constructor (and a bad `date-grain`/`output-style` word) to a payload-free
//! [`PrefArgError`] *without* echoing the offending value — the value is
//! untrusted input the store refuses to persist, and echoing it into a
//! terminal message would undo that refusal. Mirrors `contracts/args.rs`.
//!
//! `unset` takes no value, so it has no value to build; it reads the kind's
//! `required_scope()` from a *representative* value (see [`representative`]),
//! never stored. saya-types exposes no kind→scope map, and the spec forbids
//! re-deriving the rule, so the representative is the way to read the value's
//! own requirement without changing saya-types.

use thiserror::Error;

use crate::cli::PreferenceKindArg;
use saya_types::{DateGrain, OutputStyle, PreferenceValue, ScopeRequirement};

/// The stable kind string for a `PreferenceKindArg` — the same discriminator a
/// built value's `kind()` returns, available without constructing a value.
pub(super) fn kind_str(kind: PreferenceKindArg) -> &'static str {
    match kind {
        PreferenceKindArg::Timezone => "timezone",
        PreferenceKindArg::DateGrain => "date_grain",
        PreferenceKindArg::OutputStyle => "output_style",
        PreferenceKindArg::DefaultProfile => "default_profile",
    }
}

/// Builds the typed value for `set`. A `ContractError` (or a bad grain/style
/// word) is mapped to [`PrefArgError::InvalidValue`] *without* the value.
pub(super) fn build_value(
    kind: PreferenceKindArg,
    value: &str,
) -> Result<PreferenceValue, PrefArgError> {
    match kind {
        PreferenceKindArg::Timezone => {
            PreferenceValue::timezone(value).map_err(|_| PrefArgError::InvalidValue)
        }
        PreferenceKindArg::DateGrain => Ok(PreferenceValue::date_grain(
            parse_grain(value).ok_or(PrefArgError::InvalidValue)?,
        )),
        PreferenceKindArg::OutputStyle => Ok(PreferenceValue::output_style(
            parse_style(value).ok_or(PrefArgError::InvalidValue)?,
        )),
        PreferenceKindArg::DefaultProfile => {
            PreferenceValue::default_profile(value).map_err(|_| PrefArgError::InvalidValue)
        }
    }
}

/// A representative value of `kind`, used only to read `required_scope()` and
/// `kind()` for `unset`. The string-carrying variants use a minimal well-shaped
/// placeholder; the value is never persisted.
pub(super) fn representative(kind: PreferenceKindArg) -> PreferenceValue {
    match kind {
        PreferenceKindArg::Timezone => {
            PreferenceValue::timezone("UTC").expect("UTC is well-shaped")
        }
        PreferenceKindArg::DateGrain => PreferenceValue::date_grain(DateGrain::Day),
        PreferenceKindArg::OutputStyle => PreferenceValue::output_style(OutputStyle::Table),
        PreferenceKindArg::DefaultProfile => {
            PreferenceValue::default_profile("x").expect("single char is well-shaped")
        }
    }
}

/// The scope a kind requires, read from a representative value's own
/// `required_scope` — not a re-derived rule table.
pub(super) fn required_scope(kind: PreferenceKindArg) -> ScopeRequirement {
    representative(kind).required_scope()
}

fn parse_grain(value: &str) -> Option<DateGrain> {
    match value.trim() {
        "day" => Some(DateGrain::Day),
        "week" => Some(DateGrain::Week),
        "month" => Some(DateGrain::Month),
        "quarter" => Some(DateGrain::Quarter),
        "year" => Some(DateGrain::Year),
        _ => None,
    }
}

fn parse_style(value: &str) -> Option<OutputStyle> {
    match value.trim() {
        "table" => Some(OutputStyle::Table),
        "compact" => Some(OutputStyle::Compact),
        "narrative" => Some(OutputStyle::Narrative),
        _ => None,
    }
}

/// Errors from preferences argument parsing. Payload-free: a bad value is
/// untrusted input the store refuses to persist, and no variant carries it.
/// `ScopeConflict` names the scope the value *requires* (not the value), so the
/// user learns which scope to use without the offered `--profile` leaking.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum PrefArgError {
    #[error("preference value is invalid")]
    InvalidValue,
    #[error("preference requires a {0} scope")]
    ScopeConflict(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL: &str = "SENTINELVALUE";

    #[test]
    fn build_value_timezone_shape() {
        let v = build_value(PreferenceKindArg::Timezone, "Europe/London").unwrap();
        assert_eq!(v.kind(), "timezone");
    }

    #[test]
    fn build_value_grain_words() {
        for (word, grain) in [
            ("day", DateGrain::Day),
            ("week", DateGrain::Week),
            ("month", DateGrain::Month),
            ("quarter", DateGrain::Quarter),
            ("year", DateGrain::Year),
        ] {
            let v = build_value(PreferenceKindArg::DateGrain, word).unwrap();
            assert_eq!(v, PreferenceValue::date_grain(grain));
        }
    }

    #[test]
    fn build_value_style_words() {
        for (word, style) in [
            ("table", OutputStyle::Table),
            ("compact", OutputStyle::Compact),
            ("narrative", OutputStyle::Narrative),
        ] {
            let v = build_value(PreferenceKindArg::OutputStyle, word).unwrap();
            assert_eq!(v, PreferenceValue::output_style(style));
        }
    }

    #[test]
    fn build_value_default_profile_name() {
        let v = build_value(PreferenceKindArg::DefaultProfile, "warehouse").unwrap();
        assert_eq!(v.kind(), "default_profile");
    }

    #[test]
    fn invalid_value_never_echoes_input() {
        let bad = build_value(PreferenceKindArg::Timezone, &format!("{SENTINEL}!")).unwrap_err();
        assert_eq!(bad, PrefArgError::InvalidValue);
        assert!(!format!("{bad}").contains(SENTINEL));
        let bad = build_value(PreferenceKindArg::DateGrain, SENTINEL).unwrap_err();
        assert_eq!(bad, PrefArgError::InvalidValue);
        assert!(!format!("{bad}").contains(SENTINEL));
        let bad = build_value(PreferenceKindArg::OutputStyle, SENTINEL).unwrap_err();
        assert_eq!(bad, PrefArgError::InvalidValue);
        assert!(!format!("{bad}").contains(SENTINEL));
        let bad = build_value(
            PreferenceKindArg::DefaultProfile,
            &format!("{SENTINEL}\u{0}"),
        )
        .unwrap_err();
        assert_eq!(bad, PrefArgError::InvalidValue);
        assert!(!format!("{bad}").contains(SENTINEL));
    }

    #[test]
    fn required_scope_matches_value_scope() {
        assert_eq!(
            required_scope(PreferenceKindArg::Timezone),
            ScopeRequirement::Profile
        );
        assert_eq!(
            required_scope(PreferenceKindArg::DateGrain),
            ScopeRequirement::Profile
        );
        assert_eq!(
            required_scope(PreferenceKindArg::OutputStyle),
            ScopeRequirement::Global
        );
        assert_eq!(
            required_scope(PreferenceKindArg::DefaultProfile),
            ScopeRequirement::Global
        );
    }
}
