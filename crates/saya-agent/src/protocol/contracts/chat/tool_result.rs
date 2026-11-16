use serde::{Deserialize, Serialize};

/// The value-free shape of a query tool's result, persisted per tool call so a
/// saved session can show what ran without replaying it: the row count and the
/// column names. **No cell value is ever stored** — `result_shape_of` builds
/// this from a result's `row_count` and `columns` keys alone, never from
/// `rows`. `None` on a [`ToolMetadata`] marks a call that produced no
/// row-shaped result (a denied call, a non-query tool, or a failed query).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolResultShape {
    /// Rows the connector reported (a floor when the result was truncated).
    pub row_count: u64,
    /// Column names, in select order. Empty only for a result that carried no
    /// column names; an empty *result* still names its columns.
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolMetadata {
    pub name: String,
    pub status: String,
    /// The tool-call arguments serialized to JSON exactly as the model sent
    /// them. For a SQL tool that is `{"sql": "...", ...}`, so the statement the
    /// agent ran lives here. The SQL text is already user-visible in the
    /// transcript and on the `--format ndjson` event stream, so persisting it
    /// discloses nothing new; the store layer redacts credential-shaped
    /// substrings in it the same way it redacts a user turn. Result rows are
    /// never stored — only [`Self::result_shape`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub arguments: String,
    /// The value-free shape of a query result: row count and column names.
    /// `None` for tools that did not return a row-shaped result. Carries no
    /// cell values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_shape: Option<ToolResultShape>,
}
