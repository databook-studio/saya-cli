mod definitions;
mod dispatch;
mod edit;
mod glob;
mod grep;
mod list;
mod read;
mod validation;
mod write;

pub(super) mod checkpoint;
pub(super) mod redaction_guard;

pub(in crate::agent::tools) use definitions::append_definitions;
pub(in crate::agent::tools) use dispatch::execute as execute_tool;
pub(in crate::agent::tools) use validation::{is_tool_name, validate_arguments};

#[cfg(test)]
pub(crate) use edit::WORKSPACE_EDIT_MAX_BYTES;
#[cfg(test)]
pub(crate) use glob::{WORKSPACE_GLOB_MAX_MATCHES, WORKSPACE_GLOB_MAX_VISITED};
#[cfg(test)]
pub(crate) use grep::{
    WORKSPACE_GREP_MAX_FILE_BYTES, WORKSPACE_GREP_MAX_LINE_BYTES, WORKSPACE_GREP_MAX_MATCHES,
    WORKSPACE_GREP_MAX_VISITED,
};
#[cfg(test)]
pub(crate) use list::WORKSPACE_LIST_MAX_ENTRIES;
#[cfg(test)]
pub(crate) use read::WORKSPACE_READ_MAX_BYTES;
pub(crate) use write::WORKSPACE_WRITE_MAX_BYTES;

pub(in crate::agent::tools) use write::{
    FILE_CHANGED_SINCE_OBSERVED, FILE_EXISTS_REQUIRES_DIGEST, MISSING_FILE_NOT_A_REPLACE,
};
