//! The baseline's prefix table: which surfaces must keep repeating their first
//! request's prefix, and which are recorded as not doing so yet.
//!
//! This is the contract half of the baseline — `baseline.rs` measures and
//! compares the row values, and these two methods report and police the table
//! the values are read as.

use super::baseline::Baseline;

/// The surfaces whose second request does NOT repeat the first request's
/// prefix today, with the reason. Every other surface must stay stable. A row
/// here that has become stable fails the test, so it is deleted rather than
/// left to rot.
pub(super) const KNOWN_UNSTABLE: &[(&str, &str)] = &[];

impl Baseline {
    pub(super) fn print(&self) {
        println!(
            "request baseline per surface (bytes after normalisation; prefix = request 2 \
             repeats request 1)"
        );
        println!(
            "{:<10}{:>20}{:>11}{:>12}{:>13}  prefix_stable",
            "surface", "first_request_bytes", "tool_count", "tools_bytes", "system_bytes"
        );
        for surface in &self.surfaces {
            println!(
                "{:<10}{:>20}{:>11}{:>12}{:>13}  {}",
                surface.surface,
                surface.first_request_bytes,
                surface.tool_count,
                surface.tools_bytes,
                surface.system_bytes,
                surface.prefix_stable
            );
        }
        for (surface, reason) in KNOWN_UNSTABLE {
            println!("known unstable: {surface} — {reason}");
        }
    }

    /// A surface stable today must stay stable, and a recorded gap must still
    /// be a gap: the table is the contract, as in `known_gaps`.
    pub(super) fn assert_prefix_posture(&self) {
        for surface in &self.surfaces {
            let recorded = KNOWN_UNSTABLE
                .iter()
                .find(|(name, _)| *name == surface.surface);
            match (surface.prefix_stable, recorded) {
                (true, None) | (false, Some(_)) => {}
                (false, None) => panic!(
                    "{}'s second request no longer repeats its first request's prefix: record \
                     it in baseline::KNOWN_UNSTABLE with the reason, or fix the surface",
                    surface.surface
                ),
                (true, Some((name, _))) => {
                    panic!("{name}'s prefix is stable again: delete its KNOWN_UNSTABLE row")
                }
            }
        }
        for (surface, reason) in KNOWN_UNSTABLE {
            assert!(
                self.surfaces.iter().any(|row| row.surface == *surface),
                "KNOWN_UNSTABLE names {surface}, which the baseline does not measure"
            );
            assert!(!reason.is_empty(), "KNOWN_UNSTABLE needs a reason");
        }
    }
}
