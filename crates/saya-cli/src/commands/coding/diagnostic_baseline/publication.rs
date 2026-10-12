//! Publication marker for diagnostic baselines and its commit path.

use super::super::{CodingError, context, create};
use super::DiagnosticDeltaState;
use saya_harness::coding::checks::MAX_DIAGNOSTIC_BASELINE_BYTES;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PUBLICATION_PREFIX: &str = "checks-diagnostic-baseline-commit-";
const PUBLICATION_VERSION: u32 = 1;
const MAX_PUBLICATION_ENTRIES: usize = 128;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BaselinePublication {
    version: u32,
    baselines: Vec<BaselinePublicationEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BaselinePublicationEntry {
    path: String,
    bytes: usize,
    sha256: saya_types::Sha256Digest,
}

pub(super) fn publication_path(records: &Path, paths: &[PathBuf]) -> Result<PathBuf, CodingError> {
    if paths.is_empty() || paths.len() > MAX_PUBLICATION_ENTRIES {
        return Err(CodingError::InvalidRequest);
    }
    let mut names = paths
        .iter()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or(CodingError::InvalidRequest)
                .map(str::to_owned)
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort_unstable();
    let digest = create::digest(names.join("\n").as_bytes())?;
    Ok(records.join(format!("{PUBLICATION_PREFIX}{}.jsonl", digest.as_str())))
}

pub(super) fn read_publication(path: &Path, paths: &[PathBuf]) -> Option<BaselinePublication> {
    let bytes = context::read(path, MAX_DIAGNOSTIC_BASELINE_BYTES).ok()?;
    let publication: BaselinePublication = serde_json::from_slice(&bytes).ok()?;
    if publication.version != PUBLICATION_VERSION
        || publication.baselines.len() != paths.len()
        || publication.baselines.len() > MAX_PUBLICATION_ENTRIES
    {
        return None;
    }
    let mut expected = paths
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect::<Vec<_>>();
    expected.sort_unstable();
    let mut actual = publication
        .baselines
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();
    actual.sort_unstable();
    if actual != expected {
        return None;
    }
    for entry in &publication.baselines {
        let path = paths.iter().find(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some(entry.path.as_str())
        })?;
        let bytes = context::read(path, MAX_DIAGNOSTIC_BASELINE_BYTES).ok()?;
        if bytes.len() != entry.bytes || create::digest(&bytes).ok()? != entry.sha256 {
            return None;
        }
    }
    Some(publication)
}

pub(in crate::commands::coding) fn commit_pending(
    states: &mut [Option<DiagnosticDeltaState>],
) -> Result<bool, CodingError> {
    let pending = states
        .iter()
        .filter_map(Option::as_ref)
        .filter(|state| state.pending())
        .count();
    let comparable = states.iter().filter_map(Option::as_ref).count();
    if pending == 0 || pending != comparable {
        return Ok(false);
    }
    let publication = states
        .iter()
        .filter_map(Option::as_ref)
        .find_map(|state| match state {
            DiagnosticDeltaState::BaselinePending { publication, .. } => Some(publication.clone()),
            _ => None,
        })
        .ok_or(CodingError::InvalidRequest)?;
    let mut entries = Vec::new();
    let mut written = Vec::new();
    for state in states.iter().filter_map(Option::as_ref) {
        let DiagnosticDeltaState::BaselinePending {
            path,
            publication: state_publication,
            record,
            ..
        } = state
        else {
            return Err(CodingError::InvalidRequest);
        };
        if *state_publication != publication {
            return Err(CodingError::InvalidRequest);
        }
        let bytes = record.to_bytes().map_err(|_| CodingError::InvalidRequest)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(CodingError::InvalidRequest)?;
        entries.push(BaselinePublicationEntry {
            path: name.to_owned(),
            bytes: bytes.len(),
            sha256: create::digest(&bytes)?,
        });
        if let Err(error) = context::write_new(path, &bytes, MAX_DIAGNOSTIC_BASELINE_BYTES) {
            for path in written {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
        written.push(path.clone());
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let marker = BaselinePublication {
        version: PUBLICATION_VERSION,
        baselines: entries,
    };
    let bytes = serde_json::to_vec(&marker).map_err(CodingError::owner)?;
    if let Err(error) = context::write_new(&publication, &bytes, MAX_DIAGNOSTIC_BASELINE_BYTES) {
        for path in written {
            let _ = std::fs::remove_file(path);
        }
        return Err(error);
    }
    for state in states.iter_mut().filter_map(Option::as_mut) {
        if let DiagnosticDeltaState::BaselinePending { committed, .. } = state {
            *committed = true;
        }
    }
    Ok(true)
}
