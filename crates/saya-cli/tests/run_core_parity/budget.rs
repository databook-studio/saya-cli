//! The checked-in request budgets: how many bytes each surface may send on its
//! first request, before the tool count and the two byte figures it is made of.
//!
//! A golden records today's numbers and any reviewer can see them move; a
//! budget is the ceiling the test *refuses* to move, so an accidental growth —
//! a tool advertised too eagerly, a paragraph added to a prompt — fails on the
//! pull request that causes it instead of after a release. Each ceiling is the
//! value in `golden/request_baseline.json` plus 5 % for a byte figure and +1
//! for `tool_count`, so ordinary churn passes and a jump does not.
//!
//! Raising a ceiling is a reviewed change: edit the row in `budget.json` and
//! write the reason in its `reason` line. `SAYA_UPDATE_GOLDENS` rewrites the
//! golden measurement only — it never touches this file, so a regenerated
//! golden cannot raise a budget.
//!
//! The surfaces are the three the golden measures — chat, job, helper — plus
//! `chat_no_database`: the same chat asked something that shows no database
//! work, which sends no database section and only the `use_database` ask. That
//! row is the PERF2 saving made permanent.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::baseline::Baseline;

/// One surface's ceilings, with the reason the numbers sit where they do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Row {
    pub(super) surface: String,
    pub(super) first_request_bytes: usize,
    pub(super) system_bytes: usize,
    pub(super) tools_bytes: usize,
    pub(super) tool_count: usize,
    /// Why this row's ceilings are where they are — filled in whenever one is
    /// raised, so the next reader knows what the headroom was spent on.
    pub(super) reason: String,
}

/// The committed budget file, one row per surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Budget {
    /// How the ceilings are derived, restated in the file itself.
    pub(super) headroom: String,
    pub(super) surfaces: Vec<Row>,
}

impl Budget {
    pub(super) fn load() -> Self {
        let path = path();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "missing request budget {}: {error}. Restore it from the commit that \
                 introduced it — a budget is never generated",
                path.display()
            )
        });
        serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("budget {} is valid JSON: {error}", path.display()))
    }

    fn row(&self, surface: &str) -> &Row {
        self.surfaces
            .iter()
            .find(|row| row.surface == surface)
            .unwrap_or_else(|| {
                panic!(
                    "the baseline measures `{surface}`, which has no row in {}: add one, with its \
                     reason, before this surface can be measured again",
                    path().display()
                )
            })
    }
}

/// The committed budget this test compares against.
pub(super) fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/run_core_parity/budget.json")
}

/// Every measured surface's four figures, each against its ceiling.
///
/// `measured_by_the_run` names every surface the test binary measures, because
/// one call here sees one measurement: a row for a surface no test measures is
/// a budget nobody enforces, and is refused.
pub(super) fn assert_within_budget(
    measured: &Baseline,
    budget: &Budget,
    measured_by_the_run: &[&str],
) {
    assert!(
        !budget.headroom.is_empty(),
        "the budget file states how its ceilings are derived"
    );
    for surface in &measured.surfaces {
        let row = budget.row(&surface.surface);
        assert!(
            !row.reason.is_empty(),
            "{}'s budget row needs a reason line",
            row.surface
        );
        for (field, value, ceiling) in [
            (
                "first_request_bytes",
                surface.first_request_bytes,
                row.first_request_bytes,
            ),
            ("system_bytes", surface.system_bytes, row.system_bytes),
            ("tools_bytes", surface.tools_bytes, row.tools_bytes),
            ("tool_count", surface.tool_count, row.tool_count),
        ] {
            assert!(
                value <= ceiling,
                "{}'s {field} is {value}, over its ceiling {ceiling} in {}. A request budget is a \
                 deliberate, reviewed change: if the growth is intended, edit that row's `{field}` \
                 (and say why in its `reason`); otherwise fix what grew — an eagerly advertised \
                 tool, or a longer prompt. SAYA_UPDATE_GOLDENS does not raise a budget.",
                surface.surface,
                path().display()
            );
        }
    }
    for row in &budget.surfaces {
        assert!(
            measured_by_the_run.contains(&row.surface.as_str()),
            "the budget names `{}`, which no test in this binary measures: rename the row or \
             measure the surface",
            row.surface
        );
    }
}
