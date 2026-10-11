//! Bounded metadata discovery and body loading for repository and user
//! skills.
//!
//! Discovery returns descriptive context only. A skill listing grants no
//! command, filesystem, network, or model authority. The body loader
//! reopens one indexed skill under the same containment and returns inert
//! text: the text never grants authority either.

#[cfg(unix)]
mod frontmatter;
#[cfg(unix)]
mod header;
mod index;
mod load;
mod load_error;
mod metadata;
mod resources;
pub use resources::{
    MAX_SKILL_RESOURCE_BYTES, MAX_SKILL_RESOURCES, MAX_SKILL_TOTAL_RESOURCE_BYTES, SkillResource,
    load_skill_resources,
};

pub use index::discover_skills;
pub use load::{MAX_SKILL_BODY_BYTES, SkillBody, load_skill_body};
pub use load_error::SkillLoadError;
pub use metadata::{SkillEntry, SkillIndex, SkillIndexError, SkillOrigin, SkillRoot};

/// Maximum configured roots: one user root and one project root.
pub const MAX_SKILL_ROOTS: usize = 2;
/// Maximum direct entries enumerated across all configured roots.
pub const MAX_SKILL_ENTRIES: usize = 256;
/// Maximum bytes in one SKILL.md frontmatter header.
pub const MAX_SKILL_HEADER_BYTES: usize = 16 * 1024;
/// Maximum aggregate frontmatter bytes read in one discovery call.
pub const MAX_SKILL_TOTAL_HEADER_BYTES: usize = 1024 * 1024;
/// Maximum UTF-8 description bytes admitted into an index entry.
pub const MAX_SKILL_DESCRIPTION_BYTES: usize = 8 * 1024;
/// Maximum bytes in the descriptive caller-provided origin label.
pub const MAX_SKILL_LABEL_BYTES: usize = 128;
