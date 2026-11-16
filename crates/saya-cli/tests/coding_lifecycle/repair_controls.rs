//! Operator follow-up of a child: the resumed job's consent, scope and lineage.
//! Direct mode has no per-child write scope (helpers share the root working
//! checkout), so the claims these modules pin are: a follow-up cannot widen the
//! child's recorded task or program grant
//! (`scope`), the operator surface redacts and replays it (`adapters`), and
//! owned cancellation does not depend on the derived status (`cancellation`).
#[path = "repair_controls/adapters.rs"]
mod adapters;
#[path = "repair_controls/cancellation.rs"]
mod cancellation;
#[path = "repair_controls/scope.rs"]
mod scope;
#[path = "repair_controls/support.rs"]
mod support;
