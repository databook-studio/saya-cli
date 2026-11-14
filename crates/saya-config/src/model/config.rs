use super::*;

pub const MAX_CONFIG_FILE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub default_profile: Option<String>,
    #[serde(default)]
    pub ai: AiFile,
    #[serde(default)]
    pub run: RunFile,
    #[serde(default)]
    pub jobs: JobsFile,
    /// The `[job]` section: the job host's own settings — the trusted program
    /// directory it launches from, the extensions it may run, and the
    /// diagnostic profile it selects. It is protected whole, so an untrusted
    /// project layer cannot retarget any of it.
    #[serde(default)]
    pub job: JobFile,
    /// The `[limits]` section: every ceiling a surface enforces, in one
    /// vocabulary. The answering sub-tables (`session`, `ask`) are protected
    /// per sub-table — see `layers/protected/limits.rs` — so a repository's
    /// `.saya/config.toml` cannot raise a ceiling the user set. `run` is
    /// ordinary, like the `[jobs]` ceilings it replaced.
    #[serde(default)]
    pub limits: LimitsFile,
    /// The `[agents]` section: the helper lane a coding job may open — the two
    /// bounds its scheduler runs under, and the registry of helper types its
    /// `helpers` tool advertises. It is protected whole, so an untrusted
    /// project layer can neither add or retarget a type nor raise a bound.
    #[serde(default)]
    pub agents: AgentsFile,
    /// The `[host_commands]` section: the unsandboxed second lane's own
    /// shaping — `pass_env`, `timeout_seconds`. User-layer only; a
    /// project-layer declaration is a typed resolve error (see `layers.rs`),
    /// because a model-writable file must never shape unsandboxed execution.
    #[serde(default)]
    pub host_commands: HostCommandsFile,
    /// The `[session_commands]` section: the user-stated deny list of
    /// bare program names. User-layer only; a project-layer declaration is a
    /// typed resolve error (see `layers.rs`), because a model-writable deny
    /// could herd the session's work off the contained doors onto the
    /// unsandboxed lane — refusal as escalation.
    #[serde(default)]
    pub session_commands: SessionCommandsFile,
    #[serde(default)]
    pub output: OutputFile,
    #[serde(default)]
    pub memory: MemoryFile,
    /// The `[instructions]` section: which instruction sources load and
    /// their byte limits. There is deliberately no path key — the personal
    /// file location is derived from the user config directory in code, so
    /// no layer can redirect it (see `instructions.rs`).
    #[serde(default)]
    pub instructions: InstructionsFile,
    /// The `[tools]` section: one allow list that narrows every agent surface.
    /// It is protected, so an untrusted project layer cannot state it — a
    /// repository-writable file must not decide what the agent may reach.
    #[serde(default)]
    pub tools: ToolsFile,
    /// The `[hooks]` section: the user's own commands at the agent's lifecycle
    /// points. It is protected whole, so an untrusted project layer cannot
    /// state it — a repository-writable file must not be able to add a command
    /// that runs on the user's host.
    #[serde(default)]
    pub hooks: HooksFile,
    /// The `[web_search]` section: the backend a coding surface's
    /// `web_search` tool asks, or nothing. Protected whole like `[tools]`: it
    /// decides whether the agent may search at all and which host the query
    /// goes to, and a repository-writable file must not decide either.
    #[serde(default)]
    pub web_search: WebSearchFile,
    /// The `[mcp_servers]` section: the Model Context Protocol servers a
    /// surface may start and ask for tools. Protected whole like `[tools]`:
    /// every entry names a program that runs on the user's host, and a
    /// repository-writable file must not be able to add, replace or retarget
    /// one.
    #[serde(default)]
    pub mcp_servers: McpServersFile,
    /// The `[permissions]` section: whether a session may enter `bypass`. It
    /// is protected whole, so an untrusted project layer cannot state it — a
    /// repository-writable file must not be able to hand itself the one
    /// consent that answers every per-call question.
    #[serde(default)]
    pub permissions: PermissionsFile,
    #[serde(default)]
    pub ui: UiFile,
}

impl ConfigFile {
    pub fn from_toml(value: &str) -> Result<Self, ConfigError> {
        if value.len() > MAX_CONFIG_FILE_BYTES {
            return Err(ConfigError::ConfigFileTooLarge {
                bytes: value.len(),
                max_bytes: MAX_CONFIG_FILE_BYTES,
            });
        }
        toml::from_str(value).map_err(|error| {
            super::retired::removed_key_hint(value)
                .map(ConfigError::Parse)
                .or_else(|| super::job_keys::misplaced_job_key(value))
                .or_else(|| agents_table_error(value))
                .or_else(|| inline_secret_hint(value).map(ConfigError::Parse))
                .or_else(|| mcp_env_secret_hint(value).map(ConfigError::Parse))
                .unwrap_or_else(|| ConfigError::Parse(safe_parse_message(value, &error)))
        })
    }

    pub fn redacted_diagnostics(&self) -> RedactedDiagnostics {
        RedactedDiagnostics::from_file(self)
    }
}

fn safe_parse_message(source: &str, error: &toml::de::Error) -> String {
    let message = error.message();
    let safe = if message.starts_with("unknown field ")
        || message.starts_with("missing field ")
        || message.starts_with("duplicate field ")
    {
        message.to_owned()
    } else if let Some((_, expected)) = message.rsplit_once(", expected ") {
        format!("invalid TOML value, expected {expected}")
    } else {
        "invalid TOML input".to_owned()
    };
    let Some(span) = error.span() else {
        return safe;
    };
    let prefix = source.get(..span.start).unwrap_or_default();
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map_or(1, |line| line.chars().count() + 1);
    format!("{safe} at line {line}, column {column}")
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionsFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, DatabaseProfile>,
}

impl ConnectionsFile {
    pub fn from_toml(value: &str) -> Result<Self, ConfigError> {
        // `DatabaseProfile` carries `#[serde(deny_unknown_fields)]`, so an
        // unknown per-profile key (e.g. a typo'd `sslmodee`) is rejected here
        // by serde itself, against the type's own field set — no shadow list.
        toml::from_str(value).map_err(|error| {
            super::retired::removed_profile_hint(value)
                .or_else(|| inline_secret_hint(value))
                .map(ConfigError::Parse)
                .unwrap_or_else(|| ConfigError::Parse(error.to_string()))
        })
    }
}

/// Names the first shape mistake in `[agents]`, if any. The table holds two
/// settings beside the helper types and tells them apart by shape: a reserved
/// key carries a whole number, every other key carries a table. One serde struct
/// cannot say that — a named map beside named scalars costs the entries their
/// `deny_unknown_fields` — so the mistakes serde would report against a struct
/// the user never wrote are named here instead, from the text, the way a retired
/// key is.
fn agents_table_error(raw: &str) -> Option<ConfigError> {
    let value: toml::Value = toml::from_str(raw).ok()?;
    let table = value.as_table()?.get("agents")?.as_table()?;
    for (key, entry) in table {
        if crate::agents::RESERVED_AGENT_SETTINGS.contains(&key.as_str()) {
            if !entry.is_integer() {
                return Some(ConfigError::InvalidAgentType {
                    name: key.clone(),
                    reason: format!(
                        "this name is reserved for the [agents] setting agents.{key}, \
                         which must be a whole number"
                    ),
                });
            }
            continue;
        }
        if !entry.is_table() {
            return Some(ConfigError::InvalidAgentType {
                name: key.clone(),
                reason: format!("a helper type is a table: write [agents.{key}] instead"),
            });
        }
    }
    None
}

/// Secret-bearing keys that must hold a *reference* (`{ env =... }`), never
/// an inline value. A plain string here is the most common config mistake and
/// serde's untagged-enum error for it is undiagnosable — replace it with the
/// field, the location, and the fix.
const SECRET_KEYS: &[&str] = &["password", "ssl_ca", "api_key", "private_key", "passphrase"];

/// Names the first `[mcp_servers.<name>.env]` value written as a literal.
/// Serde reports only "invalid type: string" against a field the user never
/// wrote by that name; the fix — a reference — is what has to be said, so it
/// is read from the text like the other shape hints above.
fn mcp_env_secret_hint(raw: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(raw).ok()?;
    let servers = value.as_table()?.get("mcp_servers")?.as_table()?;
    for (name, entry) in servers {
        let Some(env) = entry.as_table()?.get("env").and_then(toml::Value::as_table) else {
            continue;
        };
        for (key, val) in env {
            if val.is_str() {
                return Some(format!(
                    "secrets are not allowed inline (mcp_servers `{name}`: env.{key}): use a \
                     reference like {{ env = \"SAYA_VAR\" }}, {{ file = \"...\" }}, or \
                     {{ keyring = \"...\" }}"
                ));
            }
        }
    }
    None
}

fn inline_secret_hint(raw: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(raw).ok()?;
    let mut hits = Vec::new();
    for (section, item) in value.as_table()?.iter() {
        let Some(item) = item.as_table() else {
            continue;
        };
        for (name, val) in item {
            if SECRET_KEYS.contains(&name.as_str()) && val.is_str() {
                hits.push(format!("{section}.{name}"));
            }
            // Nested tables ([profiles.<name>]) hold per-profile secrets.
            if let Some(nested) = val.as_table() {
                for (key, val) in nested {
                    if SECRET_KEYS.contains(&key.as_str()) && val.is_str() {
                        hits.push(format!("{section} `{name}`: {key}"));
                    }
                }
            }
            // Arrays of tables ([[ai.endpoints]]) hold per-entry secrets.
            // The location must name *which* entry — an endpoint's `name`
            // when present, its index otherwise — so the fix points at the
            // right endpoint, not at the section as a whole.
            if let Some(array) = val.as_array() {
                for (index, element) in array.iter().enumerate() {
                    let Some(entry) = element.as_table() else {
                        continue;
                    };
                    for (key, val) in entry {
                        if SECRET_KEYS.contains(&key.as_str()) && val.is_str() {
                            let which = entry
                                .get("name")
                                .and_then(toml::Value::as_str)
                                .map(|entry| format!("[{entry:?}]"))
                                .unwrap_or_else(|| format!("[{index}]"));
                            hits.push(format!("{section}.{name}{which}.{key}"));
                        }
                    }
                }
            }
        }
    }
    if hits.is_empty() {
        return None;
    }
    let locations = hits.join(", ");
    Some(format!(
        "secrets are not allowed inline ({locations}): use a reference like \
     {{ env = \"SAYA_VAR\" }}, {{ file = \"...\" }}, or {{ keyring = \"...\" }}"
    ))
}
