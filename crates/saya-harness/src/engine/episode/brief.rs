//! The episode brief: what one step's episode is told, and what it may
//! reach.
//!
//! Three derivations, each from the plan and the workspace alone — a run
//! must be reproducible from its spec and config alone (plan G3), so none
//! of them reads the environment:
//!
//! 1. The brief text: the plan state (steps in order, the step being
//!    driven), the step's goal and expected outputs, and the workspace
//!    manifest — names, sizes, digests, never bulk contents.
//! 2. The step's tool definitions: a tool outside the step's capabilities
//!    is **absent** from the list — hidden, not advertised-and-refused.
//!    Candidate-write tools are not capability-scoped, so they pass the
//!    narrowing and are stopped by the pinned limit instead.
//! 3. The step's `AgentLimits`: the step's budget becomes the ceilings, and
//!    learning is pinned off by construction rather than inherited from the
//!    user's memory mode.

mod definitions;
mod limits;
mod manifest;
mod render;
#[cfg(test)]
mod tests;

pub(super) use definitions::definitions;
pub(super) use limits::limits;
pub(super) use manifest::manifest;
pub(super) use render::render;
