//! The continuation-fact builder: the observed activity plus the persisted
//! turns become the contract's deterministic FACT part. Pure — the file
//! digest arrives through the caller's resolver, and the source digest is
//! computed over the redacted persisted turns, the same rule the store
//! applies on save.

use super::SessionObservations;
use saya_store::RedactedTurn;
use saya_types::{CheckFact, FactProvenance, FileFact, SourceBoundary, redact};
use sha2::{Digest, Sha256};

/// The deterministic FACT part of a session continuation: the source
/// boundary, the file facts, and the check facts. The narrative and the
/// user notes are other packets' concern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContinuationFacts {
    pub(crate) source: SourceBoundary,
    pub(crate) files: Vec<FileFact>,
    pub(crate) checks: Vec<CheckFact>,
}

/// Builds the FACT part of a continuation from what the session actually
/// did: the source boundary is positional over the persisted turns with a
/// digest over their redacted form, and every file and check fact carries
/// its tool provenance and observation time. `digest_of` resolves a file's
/// content digest — unresolvable or malformed output becomes no digest.
pub(crate) fn build_continuation_facts(
    state_turns: &[RedactedTurn],
    workspace_root: Option<&str>,
    profile: Option<&str>,
    observations: &SessionObservations,
    digest_of: &dyn Fn(&str) -> Option<String>,
) -> ContinuationFacts {
    ContinuationFacts {
        source: SourceBoundary {
            through_turn: state_turns.len() as u64,
            source_sha256: source_digest(state_turns),
            workspace_root: workspace_root.map(str::to_owned),
            profile: profile.map(str::to_owned),
        },
        files: observations
            .files
            .iter()
            .map(|file| FileFact {
                path: file.path.clone(),
                sha256: digest_of(&file.path).filter(|digest| is_sha256_hex(digest)),
                observed_unix_ms: file.observed_unix_ms,
                provenance: FactProvenance::ToolObserved {
                    tool: file.tool.clone(),
                },
            })
            .collect(),
        checks: observations
            .checks
            .iter()
            .map(|check| CheckFact {
                label: check.label.clone(),
                outcome: check.outcome,
                observed_unix_ms: check.observed_unix_ms,
                provenance: FactProvenance::ToolObserved {
                    tool: check.tool.clone(),
                },
            })
            .collect(),
    }
}

/// The source digest is over the REDACTED persisted turns — user and
/// assistant text through the same redaction the store applies on save — so
/// it tracks what a resumed session would replay, and two sources that
/// differ only by a secret hash alike.
pub(crate) fn source_digest(turns: &[RedactedTurn]) -> String {
    let redacted: Vec<RedactedTurn> = turns
        .iter()
        .map(|turn| RedactedTurn {
            user: redact(&turn.user),
            assistant: redact(&turn.assistant),
            database_derived: turn.database_derived,
            tools: turn.tools.clone(),
        })
        .collect();
    hex_sha256(&serde_json::to_vec(&redacted).unwrap_or_default())
}

/// Lowercase hex sha256 — the digest shape the contract enforces.
fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// A digest is 64 lowercase hex characters, or absent.
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// Wall-clock unix milliseconds, clamped non-negative: the observation
/// timestamp a live turn's collector entry carries.
pub(crate) fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}
