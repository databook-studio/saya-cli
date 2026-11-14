mod definitions;
mod delete;
mod dispatch;
mod edit;
mod excerpt;
mod names;
pub(in crate::agent::tools) use edit::MULTI_EDIT_OVERLAP_REFUSAL;
mod glob;
mod grep;
mod grep_types;
mod list;
pub(super) mod notebook;
mod read;
mod validation;
mod witness;
mod write;

pub(super) mod checkpoint;
pub(super) mod redaction_guard;

pub(in crate::agent::tools) use definitions::{
    append_definitions, append_notebook_edit, notebook_edit_last,
};
pub(in crate::agent::tools) use delete::{
    definition as delete_definition, execute as delete_owned,
};
pub(in crate::agent::tools) use dispatch::execute_with_context;
pub(in crate::agent::tools) use names::{
    canonical_name, canonicalize_tool_names, is_write_tool, prepare_coding_workspace_tools,
};
pub(in crate::agent::tools) use validation::{is_tool_name, validate_arguments};

#[cfg(test)]
pub(crate) use edit::WORKSPACE_EDIT_MAX_BYTES;
#[cfg(test)]
pub(crate) use glob::WORKSPACE_GLOB_MAX_MATCHES;
#[cfg(test)]
pub(crate) use grep::{
    WORKSPACE_GREP_MAX_FILE_BYTES, WORKSPACE_GREP_MAX_LINE_BYTES, WORKSPACE_GREP_MAX_MATCHES,
};
#[cfg(test)]
pub(crate) use list::WORKSPACE_LIST_MAX_ENTRIES;
#[cfg(test)]
pub(crate) use read::MAX_IMAGE_BYTES;
pub(crate) use read::WORKSPACE_READ_MAX_BYTES;
pub(crate) use write::WORKSPACE_WRITE_MAX_BYTES;

pub(in crate::agent::tools) use write::MISSING_FILE_NOT_A_REPLACE;

mod search_lifetime;
mod spool;
pub(in crate::agent::tools) use search_lifetime::{
    WorkspaceTasks, execute_search, validate_search,
};
pub(in crate::agent::tools) use witness::ReadWitnesses;
