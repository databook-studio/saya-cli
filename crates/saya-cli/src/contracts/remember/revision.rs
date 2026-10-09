//! The revision rule: when a re-remember revises a directive instead of deduping.

use saya_types::ClaimPayload;

/// True when `new` carries the same directive value as `old` but a *new,
/// explicitly-stated* reason — the one case `remember` revises instead of
/// dedups. The comparison is on the value-defining fields (column, role,
/// description) plus the reason: a difference in anything but the reason is a
/// genuine value change (which cannot land on the same single-valued row
/// anyway, and is a new fact for a multi-valued slot), and a difference in the
/// reason alone is a revision. Only a *present* new reason revises: re-stating
/// the same value with no reason does not erase an existing one — silence is
/// not "drop the reason," and the spec's case is a user who states a *new*
/// reason. Non-directive kinds never carry a reason, so they never revise —
/// an identical re-remember of a description is a plain duplicate. The `old`
/// payload is the stored one (possibly blanked for a tombstone, but a tombstone
/// is excluded by the caller's `Dismissed` guard).
pub(super) fn is_reason_revision(new: &ClaimPayload, old: &ClaimPayload) -> bool {
    use saya_types::ClaimPayload as P;
    match (new, old) {
        (
            P::DefaultTimeColumn {
                column: c1,
                reason: r1,
                ..
            },
            P::DefaultTimeColumn {
                column: c2,
                reason: r2,
                ..
            },
        ) => c1 == c2 && r1.is_some() && r1 != r2,
        (
            P::TableGrain {
                description: d1,
                reason: r1,
                ..
            },
            P::TableGrain {
                description: d2,
                reason: r2,
                ..
            },
        ) => d1 == d2 && r1.is_some() && r1 != r2,
        (
            P::ColumnRole {
                column: c1,
                role: rl1,
                reason: rr1,
                ..
            },
            P::ColumnRole {
                column: c2,
                role: rl2,
                reason: rr2,
                ..
            },
        ) => c1 == c2 && rl1 == rl2 && rr1.is_some() && rr1 != rr2,
        (
            P::JoinRule {
                target: t1,
                local_columns: l1,
                target_columns: g1,
                condition: cd1,
                reason: r1,
                ..
            },
            P::JoinRule {
                target: t2,
                local_columns: l2,
                target_columns: g2,
                condition: cd2,
                reason: r2,
                ..
            },
        ) => t1 == t2 && l1 == l2 && g1 == g2 && cd1 == cd2 && r1.is_some() && r1 != r2,
        (
            P::MetricDefinition {
                name: n1,
                definition: d1,
                columns: c1,
                reason: r1,
                ..
            },
            P::MetricDefinition {
                name: n2,
                definition: d2,
                columns: c2,
                reason: r2,
                ..
            },
        ) => n1 == n2 && d1 == d2 && c1 == c2 && r1.is_some() && r1 != r2,
        // Different kinds, or a non-directive kind, never carry a reason to
        // revise — not a revision.
        _ => false,
    }
}
