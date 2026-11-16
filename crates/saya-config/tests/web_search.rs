//! The `[web_search]` setting: what a trusted layer's table resolves to, and
//! what it refuses.
//!
//! Saya runs no search service of its own, so the setting's own meaning is
//! small: either a backend is configured — and then the endpoint and, for
//! `brave`, the credential reference are both stated — or it is not, and the
//! coding surfaces advertise no `web_search` at all. Every other shape is a
//! mistake, and each one is refused with the fix named rather than resolved
//! into something the user did not write.

use saya_config::{ConfigError, ConfigFile, ConnectionsFile, ResolutionInput, SecretRef, resolve};
use saya_types::SecretRef as TypesSecretRef;

fn with_user(toml: &str) -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(toml).expect("user fixture must parse"))
}

/// No table at all is the default, and it means exactly one thing: no search
/// backend, so no `web_search` is advertised anywhere.
#[test]
fn an_absent_table_configures_no_backend() {
    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolves");
    assert_eq!(resolved.web_search.backend, None);
}

/// A table that states nothing is the same as no table: the defaults are the
/// unconfigured shape, so a leftover empty section changes nothing.
#[test]
fn an_empty_table_configures_no_backend() {
    let resolved = resolve(with_user("[web_search]\n")).expect("resolves");
    assert_eq!(resolved.web_search.backend, None);
}

/// SearXNG is the operator's own instance: one endpoint, and no credential —
/// the request the tool builds carries none.
#[test]
fn searxng_resolves_to_its_endpoint() {
    let resolved = resolve(with_user(
        "[web_search]\nprovider = \"searxng\"\nendpoint = \"https://search.example.org/search\"\n",
    ))
    .expect("resolves");
    let backend = resolved
        .web_search
        .backend
        .expect("a backend was configured");
    assert_eq!(backend.name(), "searxng");
    assert_eq!(backend.endpoint(), "https://search.example.org/search");
}

/// Brave authenticates every request, so its credential is part of the
/// setting — as a reference, never a value.
#[test]
fn brave_resolves_to_its_endpoint_and_a_credential_reference() {
    let resolved = resolve(with_user(
        "[web_search]\nprovider = \"brave\"\n\
         endpoint = \"https://api.search.brave.com/res/v1/web/search\"\n\
         api_key = { env = \"SAYA_BRAVE_API_KEY\" }\n",
    ))
    .expect("resolves");
    let backend = resolved
        .web_search
        .backend
        .expect("a backend was configured");
    assert_eq!(backend.name(), "brave");
    assert_eq!(
        backend.endpoint(),
        "https://api.search.brave.com/res/v1/web/search"
    );
    assert_eq!(
        backend.credential(),
        Some(&TypesSecretRef::Env {
            env: "SAYA_BRAVE_API_KEY".into()
        })
    );
}

/// A provider with no endpoint is refused: the endpoint decides which host
/// the user's queries go to, and no default is guessed at.
#[test]
fn a_provider_without_an_endpoint_is_refused() {
    let error = resolve(with_user("[web_search]\nprovider = \"searxng\"\n"))
        .expect_err("an endpoint is required");
    assert!(
        matches!(error, ConfigError::InvalidWebSearch { .. }),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("web_search.endpoint"),
        "the refusal names the setting to fix: {error}"
    );
}

/// An endpoint or a credential with no provider is a mistake, not a silent
/// no-op: the table would otherwise look configured and search nothing.
#[test]
fn settings_without_a_provider_are_refused() {
    let error = resolve(with_user(
        "[web_search]\nendpoint = \"https://search.example.org/search\"\n",
    ))
    .expect_err("a provider is required");
    assert!(
        error.to_string().contains("web_search.provider"),
        "the refusal names the setting to fix: {error}"
    );
}

/// A provider saya does not have is refused rather than ignored: a user who
/// misspelled the backend must not be left believing they configured one.
#[test]
fn an_unknown_provider_is_refused() {
    let error = resolve(with_user(
        "[web_search]\nprovider = \"goggle\"\nendpoint = \"https://search.example.org/search\"\n",
    ))
    .expect_err("an unknown provider is refused");
    assert!(
        matches!(
            error,
            ConfigError::WebSearchProviderUnknown { ref provider } if provider == "goggle"
        ),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("searxng") && error.to_string().contains("brave"),
        "the refusal names the backends that exist: {error}"
    );
}

/// Brave without a credential is refused: the backend cannot answer without
/// one, so the tool would be advertised and fail every call.
#[test]
fn brave_without_a_credential_is_refused() {
    let error = resolve(with_user(
        "[web_search]\nprovider = \"brave\"\n\
         endpoint = \"https://api.search.brave.com/res/v1/web/search\"\n",
    ))
    .expect_err("brave needs a credential");
    assert!(
        error.to_string().contains("web_search.api_key"),
        "the refusal names the setting to fix: {error}"
    );
}

/// A credential on a searxng endpoint is refused rather than dropped: the
/// tool sends none there, so a stated key would be a setting that did nothing.
#[test]
fn a_credential_on_searxng_is_refused() {
    let error = resolve(with_user(
        "[web_search]\nprovider = \"searxng\"\n\
         endpoint = \"https://search.example.org/search\"\n\
         api_key = { env = \"SAYA_BRAVE_API_KEY\" }\n",
    ))
    .expect_err("searxng sends no credential");
    assert!(
        error.to_string().contains("web_search.api_key"),
        "the refusal names the setting to fix: {error}"
    );
}

/// The endpoint must be an absolute https URL with a host: it is the one
/// destination a search request may go to, and a relative or plain-http
/// endpoint is a mistake the fetch lane would refuse later anyway.
#[test]
fn a_non_https_or_hostless_endpoint_is_refused() {
    for endpoint in [
        "http://search.example.org/search",
        "search.example.org/search",
        "https:///search",
        "https://",
        "https://se arch.example.org/search",
    ] {
        let toml = format!("[web_search]\nprovider = \"searxng\"\nendpoint = \"{endpoint}\"\n");
        let error = resolve(with_user(&toml)).expect_err(&format!("{endpoint:?} must be refused"));
        assert!(
            matches!(error, ConfigError::InvalidWebSearch { .. }),
            "{endpoint:?} must be refused as a [web_search] mistake, got {error:?}"
        );
    }
}

/// The credential is a reference and nothing else: an inline value is refused
/// by the config file's own secret check, before this setting is even read.
#[test]
fn an_inline_credential_is_refused_at_parse() {
    let error = ConfigFile::from_toml(
        "[web_search]\nprovider = \"brave\"\n\
         endpoint = \"https://api.search.brave.com/res/v1/web/search\"\n\
         api_key = \"sk-live-literal\"\n",
    )
    .expect_err("an inline secret is refused");
    assert!(
        error.to_string().contains("secrets are not allowed inline"),
        "{error}"
    );
    assert!(
        !error.to_string().contains("sk-live-literal"),
        "the refusal must not echo the secret: {error}"
    );
}

/// The reference form round-trips as a `SecretRef`, so the resolver seam
/// every other secret in this tree uses is the one this setting uses.
#[test]
fn the_credential_is_the_shared_reference_type() {
    let file = ConfigFile::from_toml(
        "[web_search]\nprovider = \"brave\"\n\
         endpoint = \"https://api.search.brave.com/res/v1/web/search\"\n\
         api_key = { env = \"SAYA_BRAVE_API_KEY\" }\n",
    )
    .expect("parses");
    assert_eq!(
        file.web_search.api_key,
        Some(SecretRef::Env {
            env: "SAYA_BRAVE_API_KEY".into()
        })
    );
}
