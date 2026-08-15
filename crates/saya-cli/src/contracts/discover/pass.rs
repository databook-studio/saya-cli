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

#[cfg(test)]
mod property_tests {
    //! Property 7 (spec §2): no absolute path ever reaches a report. Both
    //! accepted `source` paths and rejected paths originate in [`relative_for`],
    //! which rebuilds the display path from the entry's file name under
    //! `.saya/contracts` rather than from the canonical (absolute) path — so no
    //! absolute path or root-identifying prefix can leak. The rejection reasons
    //! are constant strings that never carry a path. Pure: no filesystem, no
    //! async; [`relative_for`] only manipulates `Path`s.
    //!
    //! A path "leaks the root" when it is absolute or has the absolute root as a
    //! *path prefix* — not when the root happens to appear as a substring inside
    //! a relative component (e.g. root `/c` inside `/contracts`). The earlier
    //! substring detector was an over-strong oracle that flagged that
    //! coincidence as a leak; the prefix test is the real invariant.
    use super::{escaped_reason, per_file_bound_reason, relative_for};
    use crate::contracts::discover::TruncationBound;
    use proptest::prelude::*;
    use std::path::{Path, PathBuf};

    /// An entry name a hostile directory might place: ordinary, unicode, dots,
    /// dotdot, slashes, and absolute-looking fragments.
    fn entry_name() -> impl Strategy<Value = String> {
        prop::collection::vec(any::<char>(), 0..=20).prop_map(|chars| chars.into_iter().collect())
    }

    /// An absolute project root, the shape the real caller supplies.
    fn project_root() -> impl Strategy<Value = PathBuf> {
        prop::collection::vec("[a-z]{1,3}", 1..=4).prop_map(|parts| {
            let mut p = PathBuf::from("/");
            for part in parts {
                p.push(part);
            }
            p
        })
    }

    /// True iff `p` is absolute or begins with the absolute `root` as a path
    /// prefix — the two ways an absolute root can reach a report.
    fn leaks_root(p: &Path, root: &Path) -> bool {
        p.is_absolute() || p.starts_with(root)
    }

    proptest! {
        /// Property 7a — for any entry name and any absolute project root,
        /// `relative_for` yields a *relative* path that is not the absolute root
        /// and does not have it as a path prefix. A regression that returned the
        /// canonical (absolute) path would fail here on `is_absolute()`.
        #[test]
        fn relative_for_never_absolute(
            root in project_root(),
            name in entry_name(),
        ) {
            // The entry path the real caller passes: root/.saya/contracts/<name>.
            // Build it so `file_name()` behaves as it would in the pass.
            let entry = root.join(".saya").join("contracts").join(&name);
            let rel = relative_for(&root, &entry);
            prop_assert!(
                !leaks_root(&rel, &root),
                "absolute path or root prefix leaked into report: {:?} (root={:?})",
                rel,
                root
            );
        }

        /// Property 7b — the rejection reasons are payload-free of any path: a
        /// path needs a separator, and none of the reason strings contains one,
        /// so no filesystem location can be read from a rejection.
        #[test]
        fn rejection_reasons_carry_no_path(
            _root in project_root(),
        ) {
            let escaped = escaped_reason();
            prop_assert!(!escaped.contains('/') && !escaped.contains('\\'));

            for bound in [
                TruncationBound::BytesPerFile,
                TruncationBound::ClaimsPerFile,
            ] {
                let reason = per_file_bound_reason(bound);
                prop_assert!(!reason.contains('/') && !reason.contains('\\'));
            }
        }
    }
}
