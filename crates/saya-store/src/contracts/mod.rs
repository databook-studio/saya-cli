//! Persisted preference and admission helpers.
//!
//! The legacy `contract_*` claim store lived here until Spec G, Chunk 5
//! retired it: every read and write moved to `knowledge_items`, the four
//! `contract_*` tables were dropped (migration step 6), and the claim/evidence/
//! event modules were deleted. Two citizens survived the excision because they
//! are not claims and never were:
//!
//! - [`preferences`] — the `user_preferences` table (a setting, not a fact:
//!   no object, no fingerprint, no drift).
//! - [`admission`] — the structural payload gate the knowledge-items write
//!   path and the preference store both reuse.
//!
//! [`ForgetReason`] stays too: the live `forget` operation and the CLI layer
//! carry it, even though the legacy audit events that consumed it are gone.

pub(crate) mod admission;
mod preferences;

pub use preferences::{MAX_PREFERENCE_VALUE_BYTES, PreferenceStore};

/// A closed enum for *why* a claim was forgotten. The live `forget` operation
/// and the CLI `ForgetReasonArg` mapping consume it; the legacy audit trail
/// that once persisted it is gone, but the type remains the typed boundary the
/// command layer hands the operations layer. Add variants here instead of
/// accepting a free-text reason — a user's reason must never become a stored
/// string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgetReason {
    UserRequest,
    Incorrect,
    Obsolete,
    Privacy,
}

impl ForgetReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserRequest => "user_request",
            Self::Incorrect => "incorrect",
            Self::Obsolete => "obsolete",
            Self::Privacy => "privacy",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user_request" => Some(Self::UserRequest),
            "incorrect" => Some(Self::Incorrect),
            "obsolete" => Some(Self::Obsolete),
            "privacy" => Some(Self::Privacy),
            _ => None,
        }
    }
}

/// The current time as Unix milliseconds. Shared by the preference store and
/// the knowledge-items write path (both stamp `updated_unix_ms` on a write).
/// Lives here rather than in either consumer so neither depends on the other.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::ForgetReason;

    #[test]
    fn forget_reason_round_trips() {
        let reasons = [
            ForgetReason::UserRequest,
            ForgetReason::Incorrect,
            ForgetReason::Obsolete,
            ForgetReason::Privacy,
        ];
        for reason in reasons {
            let s = reason.as_str();
            let parsed = ForgetReason::parse(s).unwrap();
            assert_eq!(parsed, reason);
        }
        assert!(ForgetReason::parse("bogus").is_none());
    }
}
