//! Orchestration of the discovery pass: read the canonical root, apply the
//! bounds, parse each surviving file, and assemble the report.
//!
//! Bounds (plan §12): 32 files, 64 KiB per file, 1 MiB total, 128 claims per
//! file. The per-file bounds (bytes-per-file, claims-per-file) reject the
//! offending file and let the pass continue; the pass-wide bounds (files,
//! total-bytes) set `truncated_by` and stop the pass, naming what was dropped
//! — never a silent cap. This split is a deliberate reading of plan §12's
//! "exceeding a bound stops discovery": a single oversize file must not hide
//! the rest of the directory (test 7 separates the two cases), so the
//! per-file bounds reject-and-continue while only the pass-wide bounds stop.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::format;
use super::paths;
use super::{
    DiscoveredContract, DiscoveryError, DiscoveryReport, MAX_BYTES_PER_FILE, MAX_FILES,
    MAX_TOTAL_BYTES, RootError, TruncationBound, empty_report,
};

/// Discover and parse every `.saya/contracts/*.toml` under `project_root`.
///
/// A missing `.saya/contracts` is the normal case and returns an empty report.
/// Per-file failures are reported in [`DiscoveryReport::rejected`], never
/// raised; only a root that exists but is unusable is a hard
/// [`DiscoveryError`].
pub(crate) fn discover_contracts(project_root: &Path) -> Result<DiscoveryReport, DiscoveryError> {
    let root = match paths::canonical_root(project_root)? {
        Some(root) => root,
        None => return Ok(empty_report()),
    };

    let mut contracts = Vec::new();
    let mut rejected: Vec<(PathBuf, String)> = Vec::new();
    let mut total_bytes: u64 = 0;
    let mut truncated_by: Option<&'static str> = None;

    // `read_dir` order is unspecified; a shared repo is attacker-influenceable,
    // so we sort entry paths to keep *which files survive* deterministic
    // rather than letting filesystem order decide.
    let mut entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(empty_report()),
        Err(e) => return Err(DiscoveryError::Root(RootError::Read(e))),
    };
    let mut paths_vec: Vec<PathBuf> = Vec::new();
    while let Some(entry) = entries
        .next()
        .transpose()
        .map_err(|e| DiscoveryError::Root(RootError::Read(e)))?
    {
        paths_vec.push(entry.path());
    }
    paths_vec.sort();

    for path in &paths_vec {
        if contracts.len() + rejected.len() >= MAX_FILES {
            truncated_by = Some(TruncationBound::Files.as_str());
            break;
        }
        let relative = relative_for(project_root, path);
        match paths::check_entry(&root, path, &relative) {
            Ok(paths::Checked::Skip) => continue,
            Ok(paths::Checked::Escaped(rel)) => {
                rejected.push((rel, escaped_reason()));
                continue;
            }
            Ok(paths::Checked::Candidate(candidate)) => {
                let (mut file, len) = match paths::open_regular(&candidate) {
                    Ok(Some(v)) => v,
                    Ok(None) => continue,
                    Err(_) => continue,
                };
                if len > MAX_BYTES_PER_FILE as u64 {
                    rejected.push((
                        candidate.relative,
                        per_file_bound_reason(TruncationBound::BytesPerFile),
                    ));
                    continue;
                }
                let mut body = String::new();
                // metadata.len() is already bounded above; take() also defends
                // a misreported length.
                file.by_ref()
                    .take(MAX_BYTES_PER_FILE as u64)
                    .read_to_string(&mut body)
                    .map_err(|e| DiscoveryError::Root(RootError::Read(e)))?;
                total_bytes = total_bytes.saturating_add(body.len() as u64);
                if total_bytes > MAX_TOTAL_BYTES as u64 {
                    truncated_by = Some(TruncationBound::TotalBytes.as_str());
                    break;
                }
                match format::parse_file(&body) {
                    Ok(parsed) => contracts.push(to_discovered(candidate.relative, parsed)),
                    Err(e) => rejected.push((
                        candidate.relative,
                        format::reason(e, TruncationBound::ClaimsPerFile),
                    )),
                }
            }
            Err(_) => continue,
        }
    }

    Ok(DiscoveryReport {
        contracts,
        rejected,
        truncated_by,
    })
}

fn to_discovered(source: PathBuf, parsed: format::ParsedContract) -> DiscoveredContract {
    DiscoveredContract {
        source,
        object: parsed.object,
        claims: parsed.claims,
    }
}

/// The display path for a candidate: relative to the project root, never
/// absolute. We rebuild it from the entry name under `.saya/contracts` rather
/// than from the canonical (absolute) path, so no absolute path can leak into
/// `source` (test 11).
fn relative_for(project_root: &Path, entry_path: &Path) -> PathBuf {
    let name = entry_path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_default();
    let full = project_root.join(".saya").join("contracts").join(&name);
    full.strip_prefix(project_root)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".saya/contracts").join(name))
}

/// A payload-free reason for a per-file bound rejection. The bound name keeps
/// the rejection's vocabulary aligned with `truncated_by`.
fn per_file_bound_reason(bound: TruncationBound) -> String {
    match bound {
        TruncationBound::BytesPerFile => "file exceeds the per-file size limit".into(),
        TruncationBound::ClaimsPerFile => "file exceeds the per-file claims limit".into(),
        // Pass-wide bounds are reported via `truncated_by`, never through here.
        _ => bound.as_str().into(),
    }
}

fn escaped_reason() -> String {
    "path escapes the contracts root".into()
}
