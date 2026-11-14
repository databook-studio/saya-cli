//! The `[agents]` section: the helper lane a coding job may open — the two
//! bounds its scheduler runs under, and the registry of helper types its
//! `helpers` tool advertises.
//!
//! It is a top-level section, not a `[coding]` key, because a helper type is
//! not coding-only: the registry describes an agent shape, and the coding
//! surface is simply the one that opens children.

mod limits;
mod merge;
mod registry;
mod resolve;

pub use registry::{EXPLORE_AGENT, GENERAL_AGENT, HelperAgent, HelperAgents};
pub use resolve::ResolvedAgents;

pub(crate) use limits::RESERVED_AGENT_SETTINGS;
pub(crate) use merge::merge;
pub(crate) use resolve::resolve;
