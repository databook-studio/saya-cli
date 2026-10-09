use super::*;

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
    /// The `[host_commands]` section (H1): the unsandboxed second lane's own
    /// shaping — `pass_env`, `timeout_seconds`. User-layer only; a
    /// project-layer declaration is a typed resolve error (see `layers.rs`),
    /// because a model-writable file must never shape unsandboxed execution.
    #[serde(default)]
    pub host_commands: HostCommandsFile,
    /// The `[session_commands]` section (H1b): the user-stated deny list of
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
    #[serde(default)]
    pub ui: UiFile,
}

impl ConfigFile {
    pub fn from_toml(value: &str) -> Result<Self, ConfigError> {
        toml::from_str(value).map_err(|error| {
            inline_secret_hint(value)
                .map(ConfigError::Parse)
                .unwrap_or_else(|| ConfigError::Parse(error.to_string()))
        })
    }

    pub fn redacted_diagnostics(&self) -> RedactedDiagnostics {
        RedactedDiagnostics::from_file(self)
    }
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
            inline_secret_hint(value)
                .map(ConfigError::Parse)
                .unwrap_or_else(|| ConfigError::Parse(error.to_string()))
        })
    }
}

/// Secret-bearing keys that must hold a *reference* (`{ env =... }`), never
/// an inline value. A plain string here is the most common config mistake and
/// serde's untagged-enum error for it is undiagnosable — replace it with the
/// field, the location, and the fix.
const SECRET_KEYS: &[&str] = &["password", "ssl_ca", "api_key", "private_key", "passphrase"];

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
