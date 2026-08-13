use crate::contracts::records::ContractObjectId;
use saya_types::{ClaimId, ClaimOrigin, ClaimStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContractEventKind {
    Proposed,
    Confirmed,
    Edited,
    Rejected,
    Contradicted,
    MarkedStale,
    Forgotten,
    Imported,
    Exported,
}

impl ContractEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Confirmed => "confirmed",
            Self::Edited => "edited",
            Self::Rejected => "rejected",
            Self::Contradicted => "contradicted",
            Self::MarkedStale => "marked_stale",
            Self::Forgotten => "forgotten",
            Self::Imported => "imported",
            Self::Exported => "exported",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "proposed" => Some(Self::Proposed),
            "confirmed" => Some(Self::Confirmed),
            "edited" => Some(Self::Edited),
            "rejected" => Some(Self::Rejected),
            "contradicted" => Some(Self::Contradicted),
            "marked_stale" => Some(Self::MarkedStale),
            "forgotten" => Some(Self::Forgotten),
            "imported" => Some(Self::Imported),
            "exported" => Some(Self::Exported),
            _ => None,
        }
    }
}

/// Closed by design — a user's free-text reason must never become a persisted
/// string. Add variants to this enum instead.
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

/// A single event in a claim's audit trail. Carries no payload and no reason
/// text — the event kind and status transitions are the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractEvent {
    pub claim_id: ClaimId,
    pub object_id: ContractObjectId,
    pub kind: ContractEventKind,
    pub from_status: Option<ClaimStatus>,
    pub to_status: Option<ClaimStatus>,
    pub origin: ClaimOrigin,
    pub created_unix_ms: i64,
    /// Set only on a `Forgotten` event. A closed enum, never free text —
    /// which is why a deletion reason can be audited without storing what
    /// the user actually typed.
    pub reason: Option<ForgetReason>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_event_kind_round_trips() {
        let kinds = [
            ContractEventKind::Proposed,
            ContractEventKind::Confirmed,
            ContractEventKind::Edited,
            ContractEventKind::Rejected,
            ContractEventKind::Contradicted,
            ContractEventKind::MarkedStale,
            ContractEventKind::Forgotten,
            ContractEventKind::Imported,
            ContractEventKind::Exported,
        ];
        for kind in kinds {
            let s = kind.as_str();
            let parsed = ContractEventKind::parse(s).unwrap();
            assert_eq!(parsed, kind);
        }
        assert!(ContractEventKind::parse("bogus").is_none());
    }

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
