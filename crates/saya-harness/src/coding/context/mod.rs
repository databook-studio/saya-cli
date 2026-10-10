//! Original-project facts projected into a fresh, bounded child request.
//!
//! Captured instructions, explicit preferences, tasks, summaries, and opaque
//! artifact references are untrusted user-context data. They never grant
//! authority or enter `system_prompt`.

mod blocks;
mod brief;
mod limits;
mod preferences;
mod project;
mod source_block;
mod sources;

pub use brief::{ArtifactReference, ChildBrief, ChildDependency, ChildProjectionOptions};
pub use limits::{
    ChildContextError, MAX_CHILD_ARTIFACT_REFERENCES, MAX_CHILD_DEPENDENCIES,
    MAX_CHILD_DEPENDENCY_SUMMARY_BYTES, MAX_CHILD_TASK_BYTES, MAX_CONTEXT_BYTES, MAX_CONTEXT_ITEMS,
    MAX_CONTEXT_LABEL_BYTES, MAX_CONTEXT_REFERENCE_BYTES, MAX_CONTEXT_SOURCE_FACTS,
};
pub use sources::{InstructionSourceFact, OriginalProjectContext};
