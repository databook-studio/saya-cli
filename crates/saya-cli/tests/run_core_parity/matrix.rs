//! The parity matrix: the union of advertised names (first-seen order) by
//! surface, with each cell `same`, `schema-differs`, or `missing`.
//!
//! The reference for a name is the first surface (in capture order) that
//! advertises it. `same` means that surface's schema digest equals the
//! reference; `schema-differs` means it advertises the name with another
//! digest; `missing` means it does not advertise the name at all. A gap is
//! every non-`same` cell.

use std::collections::{BTreeMap, BTreeSet};

use super::{capture::Advertised, known_gaps};

/// One captured surface and its first request.
pub(super) struct SurfaceCapture {
    pub(super) name: &'static str,
    pub(super) advertised: Advertised,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Cell {
    Same,
    SchemaDiffers,
    Missing,
}

impl Cell {
    fn label(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::SchemaDiffers => "schema-differs",
            Self::Missing => "missing",
        }
    }
}

pub(super) struct Matrix {
    names: Vec<String>,
    surfaces: Vec<&'static str>,
    cells: BTreeMap<(String, &'static str), Cell>,
}

impl Matrix {
    pub(super) fn build(captures: &[SurfaceCapture]) -> Self {
        let mut names = Vec::new();
        let mut seen = BTreeSet::new();
        for capture in captures {
            for name in &capture.advertised.names {
                if seen.insert(name.clone()) {
                    names.push(name.clone());
                }
            }
        }
        let mut cells = BTreeMap::new();
        for name in &names {
            let Some(reference) = captures
                .iter()
                .find(|capture| capture.advertised.schemas.contains_key(name))
            else {
                continue;
            };
            let reference_hash = &reference.advertised.schemas[name];
            for capture in captures {
                let cell = match capture.advertised.schemas.get(name) {
                    None => Cell::Missing,
                    Some(hash) if hash == reference_hash => Cell::Same,
                    Some(_) => Cell::SchemaDiffers,
                };
                cells.insert((name.clone(), capture.name), cell);
            }
        }
        Self {
            names,
            surfaces: captures.iter().map(|capture| capture.name).collect(),
            cells,
        }
    }

    /// Every non-`same` cell, as `(name, surface, kind)`.
    pub(super) fn gaps(&self) -> Vec<(String, &'static str, Cell)> {
        self.cells
            .iter()
            .filter(|(_, cell)| **cell != Cell::Same)
            .map(|((name, surface), cell)| (name.clone(), *surface, *cell))
            .collect()
    }

    pub(super) fn print(&self) {
        println!(
            "run-core tool parity: {} names, surfaces {:?}",
            self.names.len(),
            self.surfaces
        );
        println!("reference for a name = the first surface in that order that advertises it");
        let mut header = format!("{:<32}", "tool");
        for surface in &self.surfaces {
            header.push_str(&format!("{surface:<16}"));
        }
        println!("{header}");
        for name in &self.names {
            let mut row = format!("{name:<32}");
            for surface in &self.surfaces {
                let cell = self.cells[&(name.clone(), *surface)];
                row.push_str(&format!("{:<16}", cell.label()));
            }
            println!("{row}");
        }
        println!(
            "not captured: {:?}",
            known_gaps::NOT_CAPTURED
                .iter()
                .map(|surface| (surface.surface, surface.reason))
                .collect::<Vec<_>>()
        );
    }

    pub(super) fn assert_known_gaps(
        &self,
        known: &[known_gaps::Gap],
        not_captured: &[known_gaps::NotCaptured],
    ) {
        for surface in not_captured {
            assert!(
                !self.surfaces.contains(&surface.surface),
                "{} is both captured and NOT_CAPTURED",
                surface.surface
            );
            assert!(
                !surface.reason.is_empty(),
                "{} needs a reason",
                surface.surface
            );
        }
        for gap in known {
            assert!(
                !gap.reason.is_empty(),
                "KNOWN_GAPS row for {}/{} needs a reason",
                gap.surface,
                gap.name
            );
        }
        let observed: BTreeSet<(String, &'static str, Cell)> = self.gaps().into_iter().collect();
        let expected: BTreeSet<(String, &'static str, Cell)> = known
            .iter()
            .map(|gap| (gap.name.to_owned(), gap.surface, gap.kind))
            .collect();
        let unrecorded: Vec<_> = observed.difference(&expected).cloned().collect();
        let stale: Vec<_> = expected.difference(&observed).cloned().collect();
        assert!(
            unrecorded.is_empty() && stale.is_empty(),
            "run-core tool parity changed:\n  \
             observed but not in KNOWN_GAPS (record it with a reason): {unrecorded:?}\n  \
             in KNOWN_GAPS but no longer observed (delete that row): {stale:?}"
        );
    }
}
