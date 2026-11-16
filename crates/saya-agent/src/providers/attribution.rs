//! The headers every model-provider request carries, and the two optional
//! OpenRouter attribution headers.
//!
//! OpenRouter attributes a request to an app from `HTTP-Referer` (the app's
//! URL) and `X-Title` (its display name). Saya sends both only to an
//! `openrouter.ai` host, so a request to any other endpoint is unchanged, and
//! the operator can turn them off with `[ai] app_attribution = false`.
//! `User-Agent` is not optional: every provider request says which client it
//! came from.

use super::settings::ProviderSettings;

/// The product identity every provider request carries.
pub(crate) const USER_AGENT: &str = concat!("saya-cli/", env!("CARGO_PKG_VERSION"));

/// The app URL OpenRouter lists as the referrer.
pub(crate) const APP_REFERER: &str = "https://github.com/databook-studio/saya-cli";

/// The display name OpenRouter lists in its Top Apps table.
pub(crate) const APP_TITLE: &str = "Saya CLI";

/// Adds the identity header, and — only on an `openrouter.ai` host with
/// attribution enabled — OpenRouter's two app-attribution headers. Headers are
/// appended, never replaced, so a header a request already carries survives.
pub(super) fn apply(
    request: reqwest::RequestBuilder,
    settings: &ProviderSettings,
) -> reqwest::RequestBuilder {
    let request = request.header(reqwest::header::USER_AGENT, USER_AGENT);
    if settings.app_attribution_applies() {
        request
            .header("HTTP-Referer", APP_REFERER)
            .header("X-Title", APP_TITLE)
    } else {
        request
    }
}
