//! Parsing a single `.saya/contracts/*.toml` file into typed claims.
//!
//! The file shape is a uniform `{ kind, value, column? }` per claim (plan §12),
//! which is NOT [`ClaimPayload`]'s own serde shape (that has variant-specific
//! fields like `{ column, role }` for `column_role`, and no `value` field at
//! all). So we deserialize this module's intermediate struct, then route every
//! claim through [`super::super::args::build_payload`]. That reuses the one
//! kind-word parser (`args::parse_kind`) and re-runs the fallible
//! [`ClaimPayload`] constructors — the only thing keeping oversized and
//! control-character-bearing text out of a later context block. A third
//! spelling of `time-column` in a third place is exactly the drift the shared
//! parser exists to prevent.
//!
//! The format is TOML, parsed with the workspace-pinned `toml` crate (already
//! used by `saya-config` for `config.toml` and `connections.toml`). This is the
//! first slice that reads attacker-influenceable files out of a shared repo;
//! feeding them through a pure-Rust serde parser instead of an unsafe-derived
//! YAML parser keeps that surface in the workspace's trusted dependency set.

use serde::Deserialize;
use thiserror::Error;

use super::super::args::{QualifiedName, build_payload, parse_kind, parse_qualified};
use super::bounds::MAX_CLAIMS_PER_FILE;
use crate::cli::ClaimKindArg;
use saya_types::ClaimPayload;

/// The only version this slice understands. An unknown version is an error
/// naming this one — stored with the data, not read from the build.
const SUPPORTED_VERSION: u32 = 1;

/// Top-level shape of a contract file. `deny_unknown_fields` makes an unknown
/// field an error rather than a silent miss: a silently ignored field is a
/// contract a human believes is in force and is not (plan §12).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractFile {
    version: u32,
    object: String,
    #[serde(default)]
    claims: Vec<ClaimEntry>,
}

/// One claim in the file's uniform shape. `column` is optional and only
/// meaningful for column-scoped kinds; `build_payload` enforces that.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimEntry {
    kind: String,
    value: String,
    #[serde(default)]
    column: Option<String>,
}

/// A parsed, validated contract held in memory for the report. Nothing here is
/// imported into the store (that is 6b, behind an explicit command + dry run).
#[derive(Debug)]
pub(crate) struct ParsedContract {
    pub object: QualifiedName,
    pub claims: Vec<ClaimPayload>,
}

/// A parse failure for a single file. The reason is payload-free of untrusted
/// content: it carries field names and supported-version numbers, never a
/// claim value the store would refuse to persist.
#[derive(Debug, Error)]
pub(crate) enum ParseError {
    #[error("unsupported version {found}; only version {supported} is supported")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("malformed qualified name; expected catalog.schema.object")]
    MalformedObject,
    #[error("unknown claim kind {kind:?}")]
    UnknownKind { kind: String },
    #[error("claim value is invalid")]
    InvalidClaim,
    #[error("more than {limit} claims in one file")]
    TooManyClaims { limit: usize },
    #[error("could not parse TOML: {message}")]
    Toml { message: String },
}

/// Parse the file body into a validated contract.
pub(crate) fn parse_file(body: &str) -> Result<ParsedContract, ParseError> {
    let file: ContractFile = toml::from_str(body).map_err(|e| ParseError::Toml {
        message: toml_reason(e),
    })?;
    if file.version != SUPPORTED_VERSION {
        return Err(ParseError::UnsupportedVersion {
            found: file.version,
            supported: SUPPORTED_VERSION,
        });
    }
    let object = parse_qualified(&file.object).map_err(|_| ParseError::MalformedObject)?;
    let mut claims = Vec::with_capacity(file.claims.len().min(MAX_CLAIMS_PER_FILE));
    for entry in file.claims {
        if claims.len() >= MAX_CLAIMS_PER_FILE {
            return Err(ParseError::TooManyClaims {
                limit: MAX_CLAIMS_PER_FILE,
            });
        }
        claims.push(parse_entry(&entry)?);
    }
    Ok(ParsedContract { object, claims })
}

fn parse_entry(entry: &ClaimEntry) -> Result<ClaimPayload, ParseError> {
    let kind = parse_kind(&entry.kind).ok_or_else(|| ParseError::UnknownKind {
        // The kind word is a vocabulary token, not a claim value; naming it in
        // an error is how a human finds their typo. It is not the user data the
        // payload constructors exist to keep out.
        kind: entry.kind.clone(),
    })?;
    let column = entry
        .column
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    // build_payload runs the fallible ClaimPayload constructors and refuses
    // oversized / control-character text. We map every failure to the same
    // payload-free `InvalidClaim` so the offending value never reaches a
    // rendered reason — the same policy args.rs uses for the CLI.
    build_payload(kind, entry.value.as_str(), column).map_err(|_| ParseError::InvalidClaim)
}

/// Reduce a toml parse error to a single payload-free line. The `toml` crate's
/// `Display` is multi-line (it renders a span and caret), and the unknown-field
/// name lands on the *last* line — so taking the first line of `to_string()`
/// would drop the very field name a human needs to find their typo.
/// `Error::message()` returns that reason alone on one line:
/// `unknown field `bogus`, expected ...` — it names the field and never echoes
/// a surrounding claim value, which is the payload-free contract the rejected
/// list relies on.
fn toml_reason(error: toml::de::Error) -> String {
    error.message().to_string()
}

/// Render a [`ParseError`] as a payload-free reason string for the rejected
/// list. The `Display` impls are already payload-free (they carry field names
/// and supported-version numbers, never a claim value). The `claims_bound`
/// argument names the per-file claims bound for the `TooManyClaims` case, so
/// the reason string stays in one vocabulary with the pass-stopping bounds.
pub(crate) fn reason(error: ParseError, claims_bound: super::bounds::TruncationBound) -> String {
    match error {
        ParseError::TooManyClaims { limit } => {
            format!(
                "{} exceeded: more than {limit} claims in one file",
                claims_bound.as_str()
            )
        }
        other => other.to_string(),
    }
}

// Compile-time anchor: build_payload and the two parsers are the shared args
// surface this module reuses. If they move or change shape, this fails to
// compile — keeping the "one kind-word parser" invariant structural.
const _: fn() = || {
    let _ = build_payload
        as fn(
            ClaimKindArg,
            &str,
            Option<&str>,
        ) -> Result<ClaimPayload, super::super::args::ArgError>;
    let _ = parse_kind as fn(&str) -> Option<ClaimKindArg>;
    let _ = parse_qualified as fn(&str) -> Result<QualifiedName, super::super::args::ArgError>;
};
