//! The `[mcp_servers.<name>]` table: the Model Context Protocol servers a
//! surface may start and ask for tools.
//!
//! Saya is already an MCP *server* (`saya mcp serve`). This table is the other
//! direction: each entry names a program the agent may run as a stdio MCP
//! server, and every tool that server advertises becomes callable as
//! `mcp__<name>__<tool>`.
//!
//! ```toml
//! [mcp_servers.files]
//! command = "mcp-server-files"
//! args = ["--root", "."]
//! env = { FILES_TOKEN = { env = "SAYA_FILES_TOKEN" } }
//! ```
//!
//! Three rules the table states once, here:
//!
//! - **Nothing is started by default.** With no table, no server exists and no
//!   surface advertises an `mcp__` tool at all.
//! - **`command` is the only shape.** A remote (HTTP) server is out of scope
//!   for this release and is refused with that reason rather than parsed and
//!   ignored — an entry that resolved to "no server" would leave the user
//!   believing they had configured one.
//! - **`env` holds references, never values.** Each value is a
//!   [`SecretRef`] like every other secret in this tree, and a literal is
//!   refused by [`crate::ConfigFile::from_toml`]'s inline-secret check before
//!   this module sees it.
//!
//! The table is protected whole (see [`crate::layers`]): a repository's
//! `.saya/config.toml` is untrusted input, and every entry here names a
//! program that runs on the user's host. A model-writable file must not be
//! able to add a server, replace one, or retarget one at a different program.
//!
//! Every bound is applied at resolve time, so the client never has to
//! re-check a command it is about to run.

mod merge;

pub(crate) use merge::merge;

use std::collections::BTreeMap;
use std::time::Duration;

use saya_types::SecretRef;
use serde::Deserialize;

use crate::ConfigError;

/// How many servers one configuration may name. Each entry is a process, and
/// they start at session or job start, so the count bounds that startup — not
/// a policy a user widens by writing more entries.
pub const MAX_MCP_SERVERS: usize = 8;

/// A server name's character ceiling. The name is half of every tool name it
/// contributes (`mcp__<name>__<tool>`), and the whole of what an operator
/// writes in `[tools] allow`.
pub const MAX_MCP_SERVER_NAME_CHARS: usize = 32;

/// The command's byte ceiling: it is one program path, and the same bound a
/// `shell` call's command carries.
pub const MAX_MCP_COMMAND_BYTES: usize = 4096;

/// The argv bounds: how many words, and how many bytes each.
pub const MAX_MCP_ARGS: usize = 64;
pub const MAX_MCP_ARG_BYTES: usize = 4096;

/// The environment bounds: how many variables one server may be handed, and
/// how long a variable's name may be.
pub const MAX_MCP_ENV_VARS: usize = 32;
pub const MAX_MCP_ENV_NAME_CHARS: usize = 64;

/// How long a server is given to answer `initialize` when its entry names no
/// timeout, and the ceiling an entry may not exceed. A server that misses its
/// bound is killed and its tools are simply absent — the failure is reported,
/// never waited on.
pub const DEFAULT_MCP_STARTUP_TIMEOUT_MS: u64 = 5_000;
pub const MAX_MCP_STARTUP_TIMEOUT_MS: u64 = 30_000;

/// The same two bounds for one `tools/call`. The default is generous because a
/// server's tool may legitimately do slow work; the ceiling keeps a hung
/// server from holding a turn forever.
pub const DEFAULT_MCP_CALL_TIMEOUT_MS: u64 = 60_000;
pub const MAX_MCP_CALL_TIMEOUT_MS: u64 = 600_000;

/// Whether `name` may be a server name. Lowercase letters, digits, `_` and
/// `-`, and never `__`: the double underscore is the namespace separator, so a
/// name carrying one would make `mcp__a__b__c` ambiguous between the server
/// `a` and the server `a__b`.
pub fn is_valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_MCP_SERVER_NAME_CHARS
        && !name.contains("__")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Raw trusted `[mcp_servers]` settings: one entry per server, keyed by the
/// name its tools are namespaced under. The table stays a plain map until the
/// layers merge.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct McpServersFile {
    pub servers: BTreeMap<String, McpServerFile>,
}

/// One `[mcp_servers.<name>]` entry.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct McpServerFile {
    /// The program to run as a stdio MCP server. Required: an entry without
    /// one names nothing to start.
    #[serde(default)]
    pub command: Option<String>,
    /// The program's arguments, verbatim — no shell parses them.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables, each a reference. A server inherits
    /// nothing else from this process: the environment is what the entry
    /// states, so a credential in the operator's shell does not reach a
    /// third-party program by accident.
    #[serde(default)]
    pub env: BTreeMap<String, SecretRef>,
    /// A remote server's URL. Refused rather than supported: only stdio
    /// servers exist in this release, and the field is here so that refusal
    /// names the fix instead of reading as a typo.
    #[serde(default)]
    pub url: Option<String>,
    /// How long this server is given to answer `initialize`. Absent means
    /// [`DEFAULT_MCP_STARTUP_TIMEOUT_MS`].
    #[serde(default)]
    pub startup_timeout_ms: Option<u64>,
    /// How long one `tools/call` may take. Absent means
    /// [`DEFAULT_MCP_CALL_TIMEOUT_MS`].
    #[serde(default)]
    pub call_timeout_ms: Option<u64>,
}

/// One server, validated: everything the client needs to start it, with every
/// bound already applied.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedMcpServer {
    name: String,
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, SecretRef>,
    startup_timeout: Duration,
    call_timeout: Duration,
}

impl ResolvedMcpServer {
    /// The name its tools are namespaced under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The program to run.
    pub fn command(&self) -> &str {
        &self.command
    }

    /// The program's arguments, verbatim.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// The extra environment, as references. Values are resolved only when
    /// the server is started.
    pub fn env(&self) -> &BTreeMap<String, SecretRef> {
        &self.env
    }

    /// How long this server is given to answer `initialize`.
    pub fn startup_timeout(&self) -> Duration {
        self.startup_timeout
    }

    /// How long one call to this server's tools may take.
    pub fn call_timeout(&self) -> Duration {
        self.call_timeout
    }
}

/// The resolved `[mcp_servers]` table. Empty — the default — is what "no MCP
/// server is configured" means everywhere downstream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedMcpServers {
    pub servers: Vec<ResolvedMcpServer>,
}

impl ResolvedMcpServers {
    /// Whether any server is configured at all. A surface with none composes
    /// no MCP client and advertises no `mcp__` tool.
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

pub(crate) fn resolve(file: &McpServersFile) -> Result<ResolvedMcpServers, ConfigError> {
    if file.servers.len() > MAX_MCP_SERVERS {
        return Err(invalid(format!(
            "the table names {} servers; the limit is {MAX_MCP_SERVERS}",
            file.servers.len()
        )));
    }
    let mut servers = Vec::with_capacity(file.servers.len());
    for (name, entry) in &file.servers {
        servers.push(resolve_one(name, entry)?);
    }
    Ok(ResolvedMcpServers { servers })
}

/// One entry, with every bound applied and every refusal naming the fix.
fn resolve_one(name: &str, entry: &McpServerFile) -> Result<ResolvedMcpServer, ConfigError> {
    if !is_valid_server_name(name) {
        return Err(invalid(format!(
            "{name:?} is not a server name: use up to {MAX_MCP_SERVER_NAME_CHARS} lowercase \
             letters, digits, `_` or `-`, and no `__` (the double underscore separates the \
             server from the tool in every tool name)"
        )));
    }
    if entry.url.is_some() {
        return Err(invalid(format!(
            "mcp_servers.{name}.url is set: remote (HTTP) MCP servers are not supported; \
             state a `command` to run a stdio server instead"
        )));
    }
    let command = entry.command.as_deref().unwrap_or_default();
    if command.is_empty() {
        return Err(invalid(format!(
            "mcp_servers.{name}.command is required: it names the program to run"
        )));
    }
    if command.len() > MAX_MCP_COMMAND_BYTES {
        return Err(invalid(format!(
            "mcp_servers.{name}.command is longer than {MAX_MCP_COMMAND_BYTES} bytes"
        )));
    }
    if entry.args.len() > MAX_MCP_ARGS {
        return Err(invalid(format!(
            "mcp_servers.{name}.args holds {} words; the limit is {MAX_MCP_ARGS}",
            entry.args.len()
        )));
    }
    if let Some(long) = entry.args.iter().find(|arg| arg.len() > MAX_MCP_ARG_BYTES) {
        return Err(invalid(format!(
            "mcp_servers.{name}.args holds a word of {} bytes; the limit is {MAX_MCP_ARG_BYTES}",
            long.len()
        )));
    }
    if entry.env.len() > MAX_MCP_ENV_VARS {
        return Err(invalid(format!(
            "mcp_servers.{name}.env holds {} variables; the limit is {MAX_MCP_ENV_VARS}",
            entry.env.len()
        )));
    }
    for key in entry.env.keys() {
        if key.is_empty()
            || key.chars().count() > MAX_MCP_ENV_NAME_CHARS
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || key.starts_with(|c: char| c.is_ascii_digit())
        {
            return Err(invalid(format!(
                "mcp_servers.{name}.env names {key:?}, which is not an environment variable \
                 name: use up to {MAX_MCP_ENV_NAME_CHARS} letters, digits and `_`, not \
                 starting with a digit"
            )));
        }
    }
    Ok(ResolvedMcpServer {
        name: name.to_owned(),
        command: command.to_owned(),
        args: entry.args.clone(),
        env: entry.env.clone(),
        startup_timeout: Duration::from_millis(checked_timeout(
            name,
            "startup_timeout_ms",
            entry.startup_timeout_ms,
            DEFAULT_MCP_STARTUP_TIMEOUT_MS,
            MAX_MCP_STARTUP_TIMEOUT_MS,
        )?),
        call_timeout: Duration::from_millis(checked_timeout(
            name,
            "call_timeout_ms",
            entry.call_timeout_ms,
            DEFAULT_MCP_CALL_TIMEOUT_MS,
            MAX_MCP_CALL_TIMEOUT_MS,
        )?),
    })
}

/// One timeout setting: absent means the default, and a stated value over the
/// ceiling is refused rather than clamped — a clamp would run under a bound
/// the user did not write and did not see.
fn checked_timeout(
    name: &str,
    field: &str,
    stated: Option<u64>,
    default: u64,
    max: u64,
) -> Result<u64, ConfigError> {
    let Some(value) = stated else {
        return Ok(default);
    };
    if value == 0 || value > max {
        return Err(invalid(format!(
            "mcp_servers.{name}.{field} = {value} is outside 1..={max}"
        )));
    }
    Ok(value)
}

fn invalid(reason: String) -> ConfigError {
    ConfigError::InvalidMcpServers { reason }
}
