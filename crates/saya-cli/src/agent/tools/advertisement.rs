//! Which tools a coding-shaped surface advertises by default, and the evidence
//! that turns the optional ones on.
//!
//! The default set is the lean one: `read`, `write`, `edit`, `glob`, `grep`,
//! `shell`, `checklist` — plus whatever the composition adds for a reason the
//! operator stated (plan mode, a background run, a notebook, MCP servers, a
//! database profile). `list` and `delete` are covered by `shell`; the web tools
//! and `helpers` are advertised only when the request itself asks for them, or
//! when the user explicitly names them in `[tools] allow` / `--tools`.
//!
//! The rule is a pure function of the request text, whether the web lane is
//! configured, and the stated allow list. Every surface that composes these
//! tools applies it at the same point — after composition, before
//! advertisement — so the coding root, a helper and a `saya run` step cannot
//! disagree about what a model sees.

use saya_config::ResolvedTools;

/// The names this rule can suppress. Every other composed name is advertised
/// exactly as composed.
const ON_EVIDENCE: &[&str] = &["list", "delete", "web_fetch", "web_search", "helpers"];

/// What the request itself states: its text, and whether the user configured a
/// web search backend (the only web setting that exists).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Evidence {
    request: String,
    web_configured: bool,
}

impl Evidence {
    pub(crate) fn new(request: impl Into<String>, web_configured: bool) -> Self {
        Self {
            request: request.into(),
            web_configured,
        }
    }

    /// Whether the request text contains an absolute http(s) URL.
    pub(crate) fn mentions_url(&self) -> bool {
        let lower = self.request.to_ascii_lowercase();
        lower.contains("http://") || lower.contains("https://")
    }

    /// Whether the request text asks for helpers: a helper, subagent or
    /// sub-agent, parallel work, a fan-out, or a delegation.
    pub(crate) fn asks_for_helpers(&self) -> bool {
        let tokens: Vec<String> = self
            .request
            .split(|character: char| !(character.is_alphanumeric() || character == '-'))
            .filter(|token| !token.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        tokens
            .iter()
            .enumerate()
            .any(|(index, token)| match token.as_str() {
                "helper" | "helpers" | "subagent" | "subagents" | "sub-agent" | "sub-agents"
                | "parallel" | "fan-out" | "fan-outs" => true,
                "fan" => tokens.get(index + 1).is_some_and(|next| next == "out"),
                other => other.starts_with("delegat"),
            })
    }

    /// Whether the user enabled web in configuration.
    pub(crate) fn web_configured(&self) -> bool {
        self.web_configured
    }
}

/// Whether `name` is advertised under `evidence` and the stated allow list.
///
/// `tools.states(name)` is the explicit override: naming a default-suppressed
/// tool in `[tools] allow` advertises it again, so a user can opt back in to
/// `list`, `delete`, the web tools, or `helpers`.
pub(crate) fn advertised(evidence: &Evidence, tools: &ResolvedTools, name: &str) -> bool {
    if !ON_EVIDENCE.contains(&name) {
        return true;
    }
    if tools.states(name) {
        return true;
    }
    match name {
        "list" | "delete" => false,
        "web_fetch" | "web_search" => evidence.mentions_url() || evidence.web_configured(),
        "helpers" => evidence.asks_for_helpers(),
        _ => true,
    }
}
