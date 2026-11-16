//! The plan contract: the ordered [`StepSpec`]s a run's engine validates and
//! binds.
//!
//! A plan is model-proposed, untrusted input. The validating constructors
//! keep hand-built plans honest, but a plan arriving as JSON skips them —
//! which is why [`RunPlan::validate`] re-checks every bound itself and is
//! the gate the engine binds a plan behind.
//!
//! A plan's step count carries no ceiling of its own: a plan reaches the
//! parser only as a model response (`saya-harness/src/engine/plan/mod.rs:70`),
//! already bounded by the model's own output cap and the stream cap
//! (`saya-agent/src/protocol/streaming.rs:16`), so no constant second-guesses it.

use crate::run::scope::is_bare_name;

mod outputs;
mod run_plan;
mod step;

pub use outputs::{Deliverable, DeliverableArtifact, MAX_OUTPUT_HINTS, OutputHint};
pub use run_plan::RunPlan;
pub use step::{MAX_STEP_CREDENTIALS, StepSpec};

/// True when `name` is safe to use as a single workspace path component: no
/// separators, no traversal, no control characters, no whitespace. The same
/// bare-name rule a runner program must satisfy, shared with
/// [`is_bare_name`](super::scope::is_bare_name) so the two cannot drift.
pub(crate) fn is_artifact_name(name: &str) -> bool {
    is_bare_name(name)
}
