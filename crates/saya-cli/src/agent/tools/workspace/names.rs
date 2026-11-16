//! Workspace tool names as the model sees them. `bash` is accepted as
//! `shell`; every other name is taken as advertised.

use saya_agent::ToolDefinition;
use serde_json::Value;

/// The advertised name used by validation, dispatch, and write admission. The
/// one alias table is `saya_config::canonical_tool_name`, so an allow list, an
/// advertisement, and a dispatch cannot disagree about which tool a word names.
pub(in crate::agent::tools) fn canonical_name(name: &str) -> &str {
    saya_config::canonical_tool_name(name)
}

/// Whether this call is one of the anchored workspace writes: the three
/// text writes, and the notebook edit that follows their rules.
pub(in crate::agent::tools) fn is_write_tool(name: &str) -> bool {
    matches!(
        canonical_name(name),
        "write" | "edit" | "delete" | "notebook_edit"
    )
}

/// Rewrite advertised names to the canonical short form.
pub(in crate::agent::tools) fn canonicalize_tool_names(tools: &mut [ToolDefinition]) {
    for tool in tools {
        let canonical = canonical_name(&tool.name);
        if canonical != tool.name {
            tool.name = canonical.to_owned();
        }
    }
}

/// Canonicalises the child workspace names. `shell` is never added here: it is
/// the one tool every surface builds from `shell_policy` — the root's
/// `RootShell`, and the helper's own, attached to the child toolset
/// by `ChildTools::attach_shell`.
pub(in crate::agent::tools) fn prepare_coding_workspace_tools(tools: &mut [ToolDefinition]) {
    canonicalize_tool_names(tools);
}

/// Map alternate argument keys onto the existing workspace schemas.
pub(in crate::agent::tools) fn normalize_arguments(name: &str, arguments: &Value) -> Value {
    let Some(object) = arguments.as_object() else {
        return arguments.clone();
    };
    let mut mapped = object.clone();
    match canonical_name(name) {
        "read" => {
            rename_key(&mut mapped, "offset", "start_line");
            rename_key(&mut mapped, "limit", "line_count");
        }
        "edit" => {
            rename_key(&mut mapped, "oldString", "old_text");
            rename_key(&mut mapped, "old_string", "old_text");
            rename_key(&mut mapped, "newString", "new_text");
            rename_key(&mut mapped, "new_string", "new_text");
        }
        "write" => rename_key(&mut mapped, "contents", "content"),
        _ => {}
    }
    Value::Object(mapped)
}

fn rename_key(object: &mut serde_json::Map<String, Value>, from: &str, to: &str) {
    if object.contains_key(to) {
        object.remove(from);
        return;
    }
    if let Some(value) = object.remove(from) {
        object.insert(to.to_owned(), value);
    }
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod names_tests;
