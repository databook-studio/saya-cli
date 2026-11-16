//! The parsed `[tools] allow` list: one validated, canonical, de-duplicated
//! set of tool names.

use std::collections::BTreeSet;

use thiserror::Error;

use super::catalog::{DATABASE_TOOLS, GOVERNED_TOOLS, allow_list_name, canonical_tool_name};

/// A stated `[tools] allow` / `--tools` list that did not name a set of
/// governed tools. Each variant carries the words to show the user, so a
/// refusal never reads as a silent no-op.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ToolsAllowError {
    /// The list was stated but named nothing. An empty list is not "allow
    /// everything": it is a typo, and treating it as the default would
    /// advertise a surface the user asked to narrow.
    #[error(
        "the tools allow list is empty: name at least one tool ({}), or omit the setting \
         to advertise every tool the surface composes",
        valid_names()
    )]
    Empty,
    /// A word that names no tool at all. The valid names ride along so the
    /// user can fix it without reading the docs.
    #[error(
        "unknown tool {name:?} in the tools allow list; valid names: {}",
        valid_names()
    )]
    Unknown { name: String },
    /// `helpers` with nothing beside it. The root delegates, so a helper runs
    /// the list minus `helpers` ([`ToolsAllow::without`]); a list of `helpers`
    /// alone derives an empty helper set, which admits nothing. Stating it is
    /// a typo, and it is refused at resolution time rather than accepted and
    /// failed by every helper at run time.
    #[error(
        "`helpers` needs at least one other tool for the helpers to use: a list of `helpers` \
         alone leaves every helper with no tools; name the tools a helper may call ({})",
        valid_names()
    )]
    HelpersAlone,
    /// A connection-tied tool. The list narrows capabilities; it cannot
    /// select a connection or turn data sharing on.
    #[error(
        "database tool {name:?} is not governed by the tools allow list: database tools follow \
         the selected connection and the data-sharing setting, not this list, so they appear \
         when the surface has both and the allow list can neither add nor remove them"
    )]
    DatabaseTool { name: String },
    /// A word that starts with the MCP namespace prefix but is not a name any
    /// server could produce. Refused by its own reason rather than as an
    /// unknown tool: the fix is the shape, not a different name.
    #[error(
        "malformed MCP tool name {name:?} in the tools allow list; state `mcp__<server>` for \
         every tool of one server, or `mcp__<server>__<tool>` for one tool, with a lowercase \
         server name of up to {} characters holding no `__`",
        crate::MAX_MCP_SERVER_NAME_CHARS
    )]
    MalformedMcpName { name: String },
}

/// The comma-separated valid names, for the unknown-name refusal.
fn valid_names() -> String {
    format!(
        "{}, or an MCP name (`mcp__<server>` / `mcp__<server>__<tool>`)",
        GOVERNED_TOOLS.join(", ")
    )
}

/// The names an allow list admits, canonical and sorted. Construction is the
/// only way to make one, so a `ToolsAllow` is always valid: every name in it
/// is a governed tool, and no name appears twice.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToolsAllow(BTreeSet<String>);

impl ToolsAllow {
    /// Validates one stated list. Duplicates are de-duplicated rather than
    /// refused — `--tools read,read` states one intent, and repeating it is
    /// not an error worth failing a launch over.
    pub fn parse<I, S>(raw: I) -> Result<Self, ToolsAllowError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut names = BTreeSet::new();
        for word in raw {
            let word = word.as_ref().trim();
            if word.is_empty() {
                continue;
            }
            let canonical = canonical_tool_name(word);
            if super::catalog::is_database_tool(canonical) {
                return Err(ToolsAllowError::DatabaseTool {
                    name: canonical.to_owned(),
                });
            }
            if !super::catalog::is_governed_tool(canonical) {
                // A word that reaches for the MCP namespace and misses is a
                // shape mistake, not a name the user misremembered: it gets
                // the shape's own refusal.
                if canonical.starts_with(super::catalog::MCP_TOOL_PREFIX) {
                    return Err(ToolsAllowError::MalformedMcpName {
                        name: word.to_owned(),
                    });
                }
                return Err(ToolsAllowError::Unknown {
                    name: word.to_owned(),
                });
            }
            names.insert(allow_list_name(canonical).to_owned());
        }
        if names.is_empty() {
            return Err(ToolsAllowError::Empty);
        }
        // `helpers` alone derives an empty helper set: the root may delegate
        // and every helper is handed nothing. The derived set is allowed to be
        // empty only when it was derived from a list that stated something for
        // the helpers to use.
        if names.len() == 1 && names.contains("helpers") {
            return Err(ToolsAllowError::HelpersAlone);
        }
        Ok(Self(names))
    }

    /// Whether this list admits `name`. The name is canonicalised first, so
    /// `bash` is admitted exactly where `shell` is — the list states the tool,
    /// not one of its spellings, nor one of its parts: the shell's readers are
    /// admitted exactly where the shell is.
    ///
    /// An MCP tool is admitted by either of the two entries that can name it:
    /// the exact `mcp__<server>__<tool>`, or the server's whole entry
    /// `mcp__<server>`. The second is what makes a list written before a
    /// server added a tool still cover the tool — the operator stated the
    /// server, which is the decision that mattered.
    pub fn allows(&self, name: &str) -> bool {
        let canonical = allow_list_name(name);
        if self.0.contains(canonical) {
            return true;
        }
        match super::catalog::mcp_server_of(canonical) {
            Some(server) => self.0.contains(server),
            None => false,
        }
    }

    /// The admitted names, canonical and sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// The admitted names as one comma-separated word, for prompts and
    /// diagnostics.
    pub fn joined(&self) -> String {
        self.names().collect::<Vec<_>>().join(",")
    }

    /// This list without `name`. Only the coding root delegates, so a helper
    /// child's set is the job's minus `helpers`. The derived set is never
    /// empty: a list that would leave nothing behind — `helpers` alone — is
    /// refused when it is parsed ([`ToolsAllowError::HelpersAlone`]).
    pub(crate) fn without(&self, name: &str) -> Self {
        let mut names = self.0.clone();
        names.remove(allow_list_name(name));
        Self(names)
    }
}

/// The valid names, for help text and refusals that list them.
pub fn governed_tool_names() -> &'static [&'static str] {
    GOVERNED_TOOLS
}

/// The connection-tied names, for help text and refusals that explain them.
pub fn database_tool_names() -> &'static [&'static str] {
    DATABASE_TOOLS
}
