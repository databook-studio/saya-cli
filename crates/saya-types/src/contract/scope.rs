//! Pairing a preference value with the scope it must be stored under.
//!
//! The rule — presentation choices are global, database-shaped choices are
//! profile-scoped — is enforced at construction here, not silently coerced at
//! storage. A caller that holds a [`Scoped`] value has already satisfied the
//! rule; the store re-checks on read, but a wrong scope never reaches a `set`
//! call through this type.

use crate::contract::error::ContractError;
use crate::contract::preference::{PreferenceScope, PreferenceValue};

/// Which scope a value must be stored under. Carried by the typed error so a
/// caller can name the scope the value requires without re-deriving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeRequirement {
    Global,
    Profile,
}

impl ScopeRequirement {
    /// The scope the error message names. Payload-free: it never echoes the
    /// value the caller tried to store.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Profile => "profile",
        }
    }

    /// True if `scope` matches this requirement.
    pub fn matches(self, scope: &PreferenceScope) -> bool {
        matches!(
            (self, scope),
            (Self::Global, PreferenceScope::Global) | (Self::Profile, PreferenceScope::Profile(_))
        )
    }
}

/// Returns the typed scope error if `scope` does not match what `value`
/// requires. The store calls this on both `set` (defence in depth — a `Scoped`
/// value already passed) and on read-back, so a row written by an older build
/// or by direct SQL can never surface a value at the wrong scope.
pub fn enforce_scope(
    scope: &PreferenceScope,
    value: &PreferenceValue,
) -> Result<(), ContractError> {
    let required = value.required_scope();
    if required.matches(scope) {
        Ok(())
    } else {
        Err(ContractError::ScopeMismatch(required.as_str()))
    }
}

/// A value paired with the scope its `required_scope` says it must be stored
/// under. Construction enforces the rule; the fields are public so the store
/// can read them, but a wrong pairing cannot be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scoped {
    pub scope: PreferenceScope,
    pub value: PreferenceValue,
}

impl Scoped {
    /// Pairs a value with the scope it requires. Returns the typed error if the
    /// caller's scope does not match the value's `required_scope`.
    pub fn new(scope: PreferenceScope, value: PreferenceValue) -> Result<Self, ContractError> {
        enforce_scope(&scope, &value)?;
        Ok(Self { scope, value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::preference::OutputStyle;

    fn profile() -> PreferenceScope {
        PreferenceScope::Profile(
            crate::ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap(),
        )
    }

    #[test]
    fn pairs_a_profile_value_with_a_profile_scope() {
        let tz = PreferenceValue::timezone("Europe/London").unwrap();
        let scoped = Scoped::new(profile(), tz).unwrap();
        assert!(matches!(scoped.scope, PreferenceScope::Profile(_)));
    }

    #[test]
    fn pairs_a_global_value_with_global_scope() {
        let style = PreferenceValue::output_style(OutputStyle::Compact);
        let scoped = Scoped::new(PreferenceScope::Global, style).unwrap();
        assert_eq!(scoped.scope, PreferenceScope::Global);
    }

    #[test]
    fn rejects_a_profile_value_at_global_scope() {
        let tz = PreferenceValue::timezone("Europe/London").unwrap();
        let err = Scoped::new(PreferenceScope::Global, tz).unwrap_err();
        assert_eq!(err, ContractError::ScopeMismatch("profile"));
    }

    #[test]
    fn rejects_a_global_value_at_profile_scope() {
        // OutputStyle is global-scoped, so pairing it with a profile scope fails.
        let style = PreferenceValue::output_style(OutputStyle::Compact);
        let err = Scoped::new(profile(), style).unwrap_err();
        assert_eq!(err, ContractError::ScopeMismatch("global"));
    }

    #[test]
    fn enforce_scope_is_the_same_check_the_store_reuses() {
        // A profile value at a global scope is refused by the free function too,
        // so the store can re-check a row on read without reconstructing a Scoped.
        let tz = PreferenceValue::timezone("Europe/London").unwrap();
        assert_eq!(
            enforce_scope(&PreferenceScope::Global, &tz),
            Err(ContractError::ScopeMismatch("profile"))
        );
        assert!(enforce_scope(&profile(), &tz).is_ok());
    }

    #[test]
    fn scope_requirement_names_are_payload_free() {
        assert_eq!(ScopeRequirement::Global.as_str(), "global");
        assert_eq!(ScopeRequirement::Profile.as_str(), "profile");
    }
}
