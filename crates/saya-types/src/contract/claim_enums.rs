use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ClaimOrigin {
    UserExplicit,
    TeamFile,
    AssistantInferred,
}

impl ClaimOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserExplicit => "user_explicit",
            Self::TeamFile => "team_file",
            Self::AssistantInferred => "assistant_inferred",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user_explicit" => Some(Self::UserExplicit),
            "team_file" => Some(Self::TeamFile),
            "assistant_inferred" => Some(Self::AssistantInferred),
            _ => None,
        }
    }

    /// Whether a claim of this origin may be stored as `Confirmed` without a
    /// per-claim review step (ADR 0002 section 4). Two origins qualify:
    ///
    /// - `UserExplicit`: the user said "remember that …" in their own words, so
    ///   the confirmation is the act of asking.
    /// - `TeamFile`: a claim read from `.saya/contracts/*.toml` that a teammate
    ///   reviewed in Git before it reached this machine. The review happened
    ///   outside saya; entering it confirmed within its declared scope is the
    ///   recorded decision, and conflicts with local claims surface per ADR
    ///   decision 3 rather than being silently dropped.
    ///
    /// Everything else (`AssistantInferred`)
    /// still enters the queue as a `Candidate`. This is the only place a
    /// non-human origin can create a confirmed claim; widening it further is a
    /// trust-model change that needs its own ADR entry.
    pub const fn may_confirm_directly(self) -> bool {
        matches!(self, Self::UserExplicit | Self::TeamFile)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ClaimStatus {
    Candidate,
    Confirmed,
    Rejected,
    Stale,
    /// Two confirmed claims disagree about a single-valued property. Nothing
    /// constructs this today, but `saya-store`'s confirm and revise rules accept
    /// it as a legal *input* state, so removing it changes which transitions are
    /// legal. Phase D retires it properly, together with the state collapse.
    Contradicted,
    Forgotten,
}

impl ClaimStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
            Self::Stale => "stale",
            Self::Contradicted => "contradicted",
            Self::Forgotten => "forgotten",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "candidate" => Some(Self::Candidate),
            "confirmed" => Some(Self::Confirmed),
            "rejected" => Some(Self::Rejected),
            "stale" => Some(Self::Stale),
            "contradicted" => Some(Self::Contradicted),
            "forgotten" => Some(Self::Forgotten),
            _ => None,
        }
    }

    /// Statuses that may reach ordinary query-building context.
    pub const fn is_recallable(self) -> bool {
        matches!(self, Self::Confirmed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ColumnRole {
    Identifier,
    Dimension,
    Measure,
    Timestamp,
    Sensitive,
}

impl ColumnRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identifier => "identifier",
            Self::Dimension => "dimension",
            Self::Measure => "measure",
            Self::Timestamp => "timestamp",
            Self::Sensitive => "sensitive",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "identifier" => Some(Self::Identifier),
            "dimension" => Some(Self::Dimension),
            "measure" => Some(Self::Measure),
            "timestamp" => Some(Self::Timestamp),
            "sensitive" => Some(Self::Sensitive),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Cardinality {
    OneToOne,
    OneToMany,
    ManyToOne,
    ManyToMany,
}

impl Cardinality {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OneToOne => "one_to_one",
            Self::OneToMany => "one_to_many",
            Self::ManyToOne => "many_to_one",
            Self::ManyToMany => "many_to_many",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "one_to_one" => Some(Self::OneToOne),
            "one_to_many" => Some(Self::OneToMany),
            "many_to_one" => Some(Self::ManyToOne),
            "many_to_many" => Some(Self::ManyToMany),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_origin_may_confirm_directly() {
        use ClaimOrigin::*;
        assert!(UserExplicit.may_confirm_directly());
        // ADR 0002 §4: a reviewed team file enters confirmed within its declared
        // scope, so TeamFile is confirmable without a per-claim review step.
        assert!(TeamFile.may_confirm_directly());
        assert!(!AssistantInferred.may_confirm_directly());
    }

    #[test]
    fn claim_status_is_recallable() {
        use ClaimStatus::*;
        assert!(Confirmed.is_recallable());
        assert!(!Candidate.is_recallable());
        assert!(!Rejected.is_recallable());
        assert!(!Stale.is_recallable());
        assert!(!Forgotten.is_recallable());
    }

    #[test]
    fn enums_as_str_and_parse() {
        for origin in &[
            ClaimOrigin::UserExplicit,
            ClaimOrigin::TeamFile,
            ClaimOrigin::AssistantInferred,
        ] {
            let s = origin.as_str();
            assert_eq!(ClaimOrigin::parse(s), Some(*origin));
        }
        assert_eq!(ClaimOrigin::parse("unknown"), None);

        for status in &[
            ClaimStatus::Candidate,
            ClaimStatus::Confirmed,
            ClaimStatus::Rejected,
            ClaimStatus::Stale,
            ClaimStatus::Forgotten,
        ] {
            let s = status.as_str();
            assert_eq!(ClaimStatus::parse(s), Some(*status));
        }
        assert_eq!(ClaimStatus::parse("unknown"), None);

        for role in &[
            ColumnRole::Identifier,
            ColumnRole::Dimension,
            ColumnRole::Measure,
            ColumnRole::Timestamp,
            ColumnRole::Sensitive,
        ] {
            let s = role.as_str();
            assert_eq!(ColumnRole::parse(s), Some(*role));
        }

        for card in &[
            Cardinality::OneToOne,
            Cardinality::OneToMany,
            Cardinality::ManyToOne,
            Cardinality::ManyToMany,
        ] {
            let s = card.as_str();
            assert_eq!(Cardinality::parse(s), Some(*card));
        }
    }

    #[test]
    fn enums_serde_round_trip() {
        for origin in &[
            ClaimOrigin::UserExplicit,
            ClaimOrigin::TeamFile,
            ClaimOrigin::AssistantInferred,
        ] {
            let json = serde_json::to_string(origin).unwrap();
            let deserialized: ClaimOrigin = serde_json::from_str(&json).unwrap();
            assert_eq!(*origin, deserialized);
        }
    }
}
