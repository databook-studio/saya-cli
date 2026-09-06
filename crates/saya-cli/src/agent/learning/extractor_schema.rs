//! Schema definitions and DTOs for structured extraction.
//!
//! Defines `ExtractedProposal`, `ProposalOrigin`, `ExtractionError`, and wire representations
//! for LLM-based knowledge extraction.

use saya_types::{ClaimOrigin, ClaimPayload, ColumnRole, KnowledgeSlot};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::turn_table::TurnObjectId;

/// Maximum proposals retained from a single extraction turn (Safety Property 3).
#[allow(dead_code)]
pub const MAX_PROPOSALS_PER_EXTRACTION: usize = 8;

/// Origin indicating whether a proposal was explicitly stated by user or inferred by assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ProposalOrigin {
    UserExplicit,
    AssistantInferred,
}

impl ProposalOrigin {
    /// Parses a string into a `ProposalOrigin` with tolerance for common synonyms.
    #[allow(dead_code)]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "user_explicit" | "user" | "explicit" => Some(Self::UserExplicit),
            "assistant_inferred" | "assistant" | "inferred" | "model" => {
                Some(Self::AssistantInferred)
            }
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserExplicit => "user_explicit",
            Self::AssistantInferred => "assistant_inferred",
        }
    }
}

impl From<ProposalOrigin> for ClaimOrigin {
    fn from(origin: ProposalOrigin) -> Self {
        match origin {
            ProposalOrigin::UserExplicit => ClaimOrigin::UserExplicit,
            ProposalOrigin::AssistantInferred => ClaimOrigin::AssistantInferred,
        }
    }
}

/// A typed, validated proposal candidate produced by structured extraction.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct ExtractedProposal {
    pub object_id: TurnObjectId,
    pub slot: KnowledgeSlot,
    pub value: ClaimPayload,
    pub origin: ProposalOrigin,
    pub confidence: f32,
}

/// Errors occurring during structured extraction response parsing.
#[derive(Debug, Error, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ExtractionError {
    #[error("failed to parse extraction JSON response: {0}")]
    JsonParse(String),
    #[error("invalid proposal payload: {0}")]
    InvalidPayload(String),
    #[error("no valid proposals found in extraction response")]
    Empty,
}

/// Wire envelope for model extraction JSON responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ExtractionResponseJson {
    #[serde(default)]
    pub proposals: Vec<RawProposalJson>,
}

/// Wire representation of a single extracted proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RawProposalJson {
    pub object_id: String,
    pub slot: String,
    pub value: String,
    /// An optional reason a directive claim carries, so a model that would
    /// argue with the claim reads its justification. Forwarded to the directive
    /// constructors only; `#[serde(default)]` so a model that omits it (or an
    /// old extraction response) decodes as no reason.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default = "default_origin")]
    pub origin: String,
    #[serde(default = "default_confidence")]
    pub confidence: Option<f32>,
    /// The qualified name of the joined table, for a `relation.join_rule`
    /// proposal. The target lives in the same profile as the object the
    /// proposal is filed against; the harness does not resolve or validate it
    /// against the schema, only stores it. `None` for every other slot.
    #[serde(default)]
    pub target: Option<String>,
    /// The local join keys, for a `relation.join_rule` proposal. May be empty
    /// for a predicate-only join. `None` for every other slot.
    #[serde(default)]
    pub local_columns: Option<Vec<String>>,
    /// The target join keys, for a `relation.join_rule` proposal, paired
    /// positionally with `local_columns`. `None` for every other slot.
    #[serde(default)]
    pub target_columns: Option<Vec<String>>,
    /// The metric's name, for a `metric.definition` proposal. `None` for every
    /// other slot.
    #[serde(default)]
    pub name: Option<String>,
    /// The underlying columns a `metric.definition` (or a `relation.join_rule`)
    /// depends on. For a metric these are the columns its formula is built
    /// from; the harness binds the fact to them so it is invalidated when one
    /// disappears. `None` for slots that take a single `value`.
    #[serde(default)]
    pub columns: Option<Vec<String>>,
}

#[allow(dead_code)]
fn default_origin() -> String {
    "assistant_inferred".to_string()
}

#[allow(dead_code)]
fn default_confidence() -> Option<f32> {
    Some(0.8)
}

/// Detects API key, token, or credential patterns in candidate values.
#[allow(dead_code)]
pub fn is_sensitive_or_credential_value(val: &str) -> bool {
    let lower = val.to_lowercase();
    let patterns = [
        "bearer ",
        "ghp_",
        "gho_",
        "github_pat_",
        "glpat-",
        "sk-",
        "sk_live_",
        "sk_test_",
        "xoxb-",
        "xoxp-",
        "-----begin private key-----",
        "-----begin rsa private key-----",
        "password=",
        "api_key=",
        "apikey=",
        "secret_key=",
    ];
    patterns.iter().any(|&p| lower.contains(p))
}

/// Parses a raw proposal into a validated `ClaimPayload`. The reason is
/// forwarded to the directive constructors only — a reason on a description or
/// alias is dropped, since those constructors accept none. A reason that fails
/// `validate_text` (too long, control chars) fails the whole payload rather
/// than being silently dropped: a model that emits an oversized reason should
/// not have it stored truncated.
///
/// The multi-field slots take more than a `value`: a `relation.join_rule`
/// carries its target table and join keys, and a `metric.definition` carries
/// its name and underlying columns. Those arrive on the optional fields of
/// [`RawProposalJson`]; a missing required field (a join rule with no target, a
/// metric with no name) is a typed error rather than a guess.
#[allow(dead_code)]
pub fn build_claim_payload(
    slot: &KnowledgeSlot,
    raw: &RawProposalJson,
) -> Result<ClaimPayload, ExtractionError> {
    let clean = raw.value.trim();
    if clean.is_empty() {
        return Err(ExtractionError::InvalidPayload("empty value".into()));
    }
    if is_sensitive_or_credential_value(clean) {
        return Err(ExtractionError::InvalidPayload(
            "sensitive credential detected".into(),
        ));
    }
    let reason = raw.reason.as_deref();

    match slot {
        KnowledgeSlot::TableDescription => ClaimPayload::table_description(clean)
            .map_err(|e| ExtractionError::InvalidPayload(e.to_string())),
        KnowledgeSlot::TableAlias => ClaimPayload::table_alias(clean)
            .map_err(|e| ExtractionError::InvalidPayload(e.to_string())),
        KnowledgeSlot::TableGrain => ClaimPayload::table_grain(clean, reason)
            .map_err(|e| ExtractionError::InvalidPayload(e.to_string())),
        KnowledgeSlot::TableDefaultTime => ClaimPayload::default_time_column(clean, reason)
            .map_err(|e| ExtractionError::InvalidPayload(e.to_string())),
        KnowledgeSlot::ColumnDescription { column } => {
            ClaimPayload::column_description(column, clean)
                .map_err(|e| ExtractionError::InvalidPayload(e.to_string()))
        }
        KnowledgeSlot::ColumnRole { column } => {
            let role = ColumnRole::parse(clean.to_lowercase().as_str()).ok_or_else(|| {
                ExtractionError::InvalidPayload(format!("unknown column role '{clean}'"))
            })?;
            ClaimPayload::column_role(column, role, reason)
                .map_err(|e| ExtractionError::InvalidPayload(e.to_string()))
        }
        KnowledgeSlot::RelationJoinRule => {
            let target = required_field(raw.target.as_deref(), "join rule target")?;
            let local_columns = raw.local_columns.clone().unwrap_or_default();
            let target_columns = raw.target_columns.clone().unwrap_or_default();
            ClaimPayload::join_rule(target, local_columns, target_columns, clean, reason)
                .map_err(|e| ExtractionError::InvalidPayload(e.to_string()))
        }
        KnowledgeSlot::MetricDefinition => {
            let name = required_field(raw.name.as_deref(), "metric name")?;
            let columns = raw.columns.clone().unwrap_or_default();
            ClaimPayload::metric_definition(name, clean, columns, reason)
                .map_err(|e| ExtractionError::InvalidPayload(e.to_string()))
        }
        // `KnowledgeSlot` is non-exhaustive; a slot this build does not know
        // how to build a payload for is rejected rather than guessed.
        _ => Err(ExtractionError::InvalidPayload(format!(
            "unsupported slot '{slot}'"
        ))),
    }
}

/// A required optional field from the raw proposal, trimmed and rejected when
/// absent or blank. Used for the fields a multi-field slot cannot omit (a join
/// rule's target, a metric's name) so the proposal fails closed rather than
/// filing a half-formed fact.
fn required_field(value: Option<&str>, what: &str) -> Result<String, ExtractionError> {
    let trimmed = value.map(str::trim).filter(|s| !s.is_empty());
    trimmed
        .map(str::to_string)
        .ok_or_else(|| ExtractionError::InvalidPayload(format!("missing {what}")))
}
