//! The `[web_search]` setting: the backend a coding surface's `web_search`
//! tool runs against, or nothing at all.
//!
//! Saya has no search service of its own, so the tool is **off until a
//! trusted layer configures it**: with no `[web_search]` table (or one that
//! names no provider) the coding surfaces advertise no `web_search` at all,
//! and no search request exists to make.
//!
//! Two backends, and the table is small because the tool is:
//!
//! ```toml
//! [web_search]
//! provider = "searxng"
//! endpoint = "https://search.example.org/search"
//!
//! [web_search]
//! provider = "brave"
//! endpoint = "https://api.search.brave.com/res/v1/web/search"
//! api_key  = { env = "SAYA_BRAVE_API_KEY" }
//! ```
//!
//! The `endpoint` is stated rather than defaulted: it decides which host the
//! user's queries go to, which is exactly the kind of setting this codebase
//! refuses to guess at. For `brave` the `api_key` is a **reference** like
//! every other secret in this tree — never a literal, and refused as one by
//! [`crate::ConfigFile::from_toml`]'s inline-secret check before this module
//! ever sees it.
//!
//! The table is protected whole (see [`crate::layers`]): a repository's
//! `.saya/config.toml` is untrusted input, and this table decides whether the
//! agent may search at all and which host it asks. A model-writable file must
//! not be able to turn egress on, nor point it at a host of its choosing.

use saya_types::SecretRef;
use serde::Deserialize;

use crate::ConfigError;

mod merge;

pub(crate) use merge::merge;

/// The most bytes one `endpoint` may carry: the same bound the fetch lane
/// puts on a URL, so an endpoint this module accepts is one the tool can
/// actually ask for.
pub const MAX_ENDPOINT_BYTES: usize = 2048;

/// Raw trusted `[web_search]` settings. Every field stays optional until the
/// layers merge, so a higher-precedence layer can override one setting at a
/// time and a table that states nothing resolves to "not configured".
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WebSearchFile {
    /// Which backend: `"searxng"` or `"brave"`. `None` is the default and
    /// means no search backend is configured.
    pub provider: Option<String>,
    /// The JSON search API to ask. Required whenever `provider` is stated.
    pub endpoint: Option<String>,
    /// The backend's credential, as a reference. Required for `brave`, which
    /// authenticates every request; refused for `searxng`, which does not.
    pub api_key: Option<SecretRef>,
}

/// The backend a `web_search` call runs against, resolved. The credential
/// stays a reference here: values are resolved only when a call is made.
#[derive(Debug, Clone, PartialEq)]
pub enum WebSearchBackend {
    /// A SearXNG instance's JSON API. The instance is the operator's own
    /// deployment, so it carries no credential.
    Searxng { endpoint: String },
    /// Brave's Web Search API, over an endpoint the operator states (Brave's
    /// own, or a gateway in front of it) and a subscription token.
    Brave {
        endpoint: String,
        api_key: SecretRef,
    },
}

impl WebSearchBackend {
    /// The configured endpoint, whichever backend it is. The one host a
    /// search request may go to.
    pub fn endpoint(&self) -> &str {
        match self {
            Self::Searxng { endpoint } | Self::Brave { endpoint, .. } => endpoint,
        }
    }

    /// The backend's name as the configuration states it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Searxng { .. } => "searxng",
            Self::Brave { .. } => "brave",
        }
    }

    /// The credential reference this backend authenticates with, or `None`
    /// where it sends none. Still a reference: the value is resolved only
    /// when a call is made.
    pub fn credential(&self) -> Option<&SecretRef> {
        match self {
            Self::Searxng { .. } => None,
            Self::Brave { api_key, .. } => Some(api_key),
        }
    }
}

/// The resolved `[web_search]` setting. `backend: None` — the default — is
/// what "no search backend is configured" means everywhere downstream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedWebSearch {
    pub backend: Option<WebSearchBackend>,
}

pub(crate) fn resolve(file: &WebSearchFile) -> Result<ResolvedWebSearch, ConfigError> {
    let Some(provider) = file.provider.as_deref() else {
        // No provider: the table is either absent or states only settings
        // that mean nothing without one. The latter is a mistake worth
        // naming rather than silently dropping.
        if file.endpoint.is_some() || file.api_key.is_some() {
            return Err(ConfigError::InvalidWebSearch {
                reason: "web_search.endpoint or web_search.api_key is set but \
                         web_search.provider is not; name the backend (\"searxng\" or \"brave\")",
            });
        }
        return Ok(ResolvedWebSearch { backend: None });
    };
    let endpoint = file
        .endpoint
        .as_deref()
        .ok_or(ConfigError::InvalidWebSearch {
            reason: "web_search.endpoint is required: it is the host a search request goes to, \
                     and no default is guessed at",
        })?;
    let endpoint = checked_endpoint(endpoint)?;
    match provider {
        "searxng" => {
            if file.api_key.is_some() {
                return Err(ConfigError::InvalidWebSearch {
                    reason: "web_search.api_key is set for provider \"searxng\", which sends no \
                             credential; remove it, or use provider \"brave\"",
                });
            }
            Ok(ResolvedWebSearch {
                backend: Some(WebSearchBackend::Searxng { endpoint }),
            })
        }
        "brave" => {
            let api_key = file.api_key.clone().ok_or(ConfigError::InvalidWebSearch {
                reason: "web_search.api_key is required for provider \"brave\": state it as a \
                             reference, e.g. { env = \"SAYA_BRAVE_API_KEY\" }",
            })?;
            Ok(ResolvedWebSearch {
                backend: Some(WebSearchBackend::Brave { endpoint, api_key }),
            })
        }
        other => Err(ConfigError::WebSearchProviderUnknown {
            provider: other.to_owned(),
        }),
    }
}

/// The endpoint as the tool will ask for it, or a typed refusal. Checked here
/// rather than at call time so a misconfigured endpoint is a config error the
/// user sees once, not a tool failure the model sees every call. This is a
/// shape check only — the fetch lane's policy is still the gate that judges
/// the URL before anything connects.
fn checked_endpoint(endpoint: &str) -> Result<String, ConfigError> {
    if endpoint.len() > MAX_ENDPOINT_BYTES {
        return Err(ConfigError::InvalidWebSearch {
            reason: "web_search.endpoint is longer than 2048 bytes",
        });
    }
    let Some(rest) = endpoint.strip_prefix("https://") else {
        return Err(ConfigError::InvalidWebSearch {
            reason: "web_search.endpoint must be an absolute https URL",
        });
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() || host.contains(char::is_whitespace) {
        return Err(ConfigError::InvalidWebSearch {
            reason: "web_search.endpoint must name a host",
        });
    }
    Ok(endpoint.to_owned())
}
