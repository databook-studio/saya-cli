//! Contract tests for the session continuation record in `saya_types`.
//!
//! A continuation is carried inside a session record and read back on resume —
//! model-adjacent, untrusted input — so `validate()` re-checks every bound no
//! matter how the record arrived, and an unknown version, unknown field, or
//! unprovenanced fact is refused, never ignored. Absence stays representable:
//! a legacy record without the field deserializes as `None`.

use saya_types::{
    CONTINUATION_VERSION, CheckFact, CheckOutcome, ContinuationError, FactProvenance, FileFact,
    GeneratedNarrative, MAX_CHECK_FACTS, MAX_CHECK_LABEL_CHARS, MAX_CONTINUATION_BYTES,
    MAX_FACT_PATH_BYTES, MAX_FILE_FACTS, MAX_NARRATIVE_BYTES, MAX_USER_NOTES_BYTES,
    NarrativeSource, SessionContinuationV1, SourceBoundary,
};
const VALID_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const TOOL: &str = "saya_test_tool";
const OBSERVED: i64 = 1_700_000_000_000;

fn boundary() -> SourceBoundary {
    SourceBoundary {
        through_turn: 12,
        source_sha256: VALID_SHA256.to_string(),
        workspace_root: Some("/work/saya".to_string()),
        profile: Some("main".to_string()),
    }
}

fn file(path: &str) -> FileFact {
    FileFact {
        path: path.to_string(),
        sha256: Some(VALID_SHA256.to_string()),
        observed_unix_ms: OBSERVED,
        provenance: FactProvenance::ToolObserved {
            tool: TOOL.to_string(),
        },
    }
}

fn check(label: &str) -> CheckFact {
    CheckFact {
        label: label.to_string(),
        outcome: CheckOutcome::Passed,
        observed_unix_ms: OBSERVED,
        provenance: FactProvenance::ToolObserved {
            tool: TOOL.to_string(),
        },
    }
}

fn narrative(text: &str) -> Option<GeneratedNarrative> {
    Some(GeneratedNarrative {
        text: text.to_string(),
        source: NarrativeSource::Summarizer,
    })
}

fn minimal() -> SessionContinuationV1 {
    SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: boundary(),
        narrative: None,
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    }
}

fn full() -> SessionContinuationV1 {
    SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: boundary(),
        narrative: narrative("profiled the tables\nnext: check the indexes"),
        user_notes: Some("keep the schema cache in mind".to_string()),
        files: vec![file("src/main.rs"), file("crates/saya-types/src/lib.rs")],
        checks: vec![check("cargo test"), check("cargo clippy")],
    }
}

fn bytes_of(record: &SessionContinuationV1) -> Vec<u8> {
    serde_json::to_vec(record).unwrap()
}

#[path = "continuation/facts.rs"]
mod facts;
#[path = "continuation/provenance.rs"]
mod provenance;
#[path = "continuation/record_wire.rs"]
mod record_wire;
#[path = "continuation/text_bounds.rs"]
mod text_bounds;
