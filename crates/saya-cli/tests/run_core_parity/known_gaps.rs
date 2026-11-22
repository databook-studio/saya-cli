//! Today's recorded differences, one row per non-`same` matrix cell.
//!
//! This table is the *contract*: the test fails when the observed gap set and
//! this table differ in either direction. A new gap means a surface gained or
//! lost a tool — record it here with the reason, or fix the surface. A row that
//! is no longer observed means the gap closed — delete the row.

use super::matrix::Cell;

/// One observed non-`same` cell.
pub(super) struct Gap {
    pub(super) surface: &'static str,
    pub(super) name: &'static str,
    pub(super) kind: Cell,
    pub(super) reason: &'static str,
}

impl Gap {
    const fn missing(surface: &'static str, name: &'static str, reason: &'static str) -> Self {
        Self {
            surface,
            name,
            kind: Cell::Missing,
            reason,
        }
    }

    const fn schema_differs(
        surface: &'static str,
        name: &'static str,
        reason: &'static str,
    ) -> Self {
        Self {
            surface,
            name,
            kind: Cell::SchemaDiffers,
            reason,
        }
    }
}

/// A surface the test cannot capture, with the one-line reason.
pub(super) struct NotCaptured {
    pub(super) surface: &'static str,
    pub(super) reason: &'static str,
}

/// Every non-`same` cell observed today, from the printed matrix.
pub(super) const KNOWN_GAPS: &[Gap] = &[
    Gap::schema_differs(
        "job",
        "schema_discovery",
        "coding surfaces narrow `connection` to the admitted profile enum; chat offers the free-form property",
    ),
    Gap::schema_differs(
        "helper",
        "schema_discovery",
        "coding surfaces narrow `connection` to the admitted profile enum; chat offers the free-form property",
    ),
    Gap::missing(
        "job",
        "request_clarification",
        "no operator is attached to the job fixture, so the coding root advertises no ask",
    ),
    Gap::missing(
        "helper",
        "request_clarification",
        "no operator is attached to the job fixture, so the helper advertises no ask",
    ),
    Gap::schema_differs(
        "job",
        "bounded_sql_query",
        "coding surfaces narrow `connection` to the admitted profile enum; chat offers the free-form property",
    ),
    Gap::schema_differs(
        "helper",
        "bounded_sql_query",
        "coding surfaces narrow `connection` to the admitted profile enum; chat offers the free-form property",
    ),
    Gap::missing(
        "job",
        "bounded_sql_query_all",
        "coding surfaces advertise only the two standard read-only database tools",
    ),
    Gap::missing(
        "helper",
        "bounded_sql_query_all",
        "coding surfaces advertise only the two standard read-only database tools",
    ),
    Gap::missing(
        "job",
        "result_shape",
        "coding surfaces advertise only the two standard read-only database tools",
    ),
    Gap::missing(
        "helper",
        "result_shape",
        "coding surfaces advertise only the two standard read-only database tools",
    ),
    Gap::missing(
        "job",
        "column_health",
        "coding surfaces advertise only the two standard read-only database tools",
    ),
    Gap::missing(
        "helper",
        "column_health",
        "coding surfaces advertise only the two standard read-only database tools",
    ),
    Gap::missing(
        "job",
        "join_check",
        "coding surfaces advertise only the two standard read-only database tools",
    ),
    Gap::missing(
        "helper",
        "join_check",
        "coding surfaces advertise only the two standard read-only database tools",
    ),
    Gap::missing(
        "job",
        "designate_answer",
        "coding surfaces advertise only the two standard read-only database tools",
    ),
    Gap::missing(
        "helper",
        "designate_answer",
        "coding surfaces advertise only the two standard read-only database tools",
    ),
    Gap::missing(
        "job",
        "contract_search",
        "coding surfaces do not compose the contract-memory tools",
    ),
    Gap::missing(
        "helper",
        "contract_search",
        "coding surfaces do not compose the contract-memory tools",
    ),
    Gap::missing(
        "job",
        "contract_read",
        "coding surfaces do not compose the contract-memory tools",
    ),
    Gap::missing(
        "helper",
        "contract_read",
        "coding surfaces do not compose the contract-memory tools",
    ),
    Gap::missing(
        "chat",
        "helpers",
        "chat has no delegation tool; only the coding root delegates",
    ),
    Gap::missing(
        "helper",
        "helpers",
        "a helper's set removes `helpers`; only the root delegates",
    ),
    Gap::missing(
        "chat",
        "web_fetch",
        "every surface's `web_fetch` is evidence-gated, and this fixture's message names no URL",
    ),
    Gap::missing(
        "helper",
        "rewind",
        "`rewind` is attached only to the job root's own checkpoint store",
    ),
];

/// Surfaces this test does not capture.
pub(super) const NOT_CAPTURED: &[NotCaptured] = &[NotCaptured {
    surface: "background",
    reason: "`saya start` dispatches the same NativeCodingJob root captured as `job`; \
             it adds no distinct provider-request surface",
}];
