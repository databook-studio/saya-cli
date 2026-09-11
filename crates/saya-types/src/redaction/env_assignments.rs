//! The env-assignment pass of [`redact`]: the credential shapes this product
//! itself creates. The run injects every resolved endpoint credential into a
//! child as `SAYA_RUN_EP_<ROLE>=<value>` — a name that matches none of the
//! `key=value` markers — so a child echoing its environment or an error
//! quoting the assignment would otherwise reach the model and disk intact.

use super::eq_ignore_ascii_case_slice;

/// The env-var prefix every resolved endpoint credential is injected under.
/// [`super::redact`] treats any `NAME=value` assignment whose name sits in
/// this namespace as credential-bearing regardless of the role name; the
/// harness's `endpoint_env_var` must generate names under the same constant,
/// so the scrubber and the injector cannot drift apart.
pub const CREDENTIAL_ENV_PREFIX: &str = "SAYA_RUN_EP_";

/// Assignment names whose final segment (underscore- or hyphen-delimited)
/// names secret material. A denylist: it covers the `NAME=value` shapes an
/// environment dump, `export`, a shell echo, or an inline error message
/// produces, with a plural suffix allowed (`SECRETS=`, `API_KEYS=`). It does
/// NOT cover a different separator (`KEY: v`, JSON `"KEY": "v"`), a value
/// written before its name, whitespace before `=`, a value that outlives its
/// boundary, or — above all — a credential under a name no word here calls
/// secret (a renamed run prefix, an obfuscated role). Denylists rot; the
/// structural defence is references-only configuration and an otherwise
/// empty child environment, and this pass only scrubs text that escaped both.
const CREDENTIAL_NAME_WORDS: [&str; 10] = [
    "apikey",
    "passphrase",
    "password",
    "passwd",
    "pass",
    "token",
    "secret",
    "credential",
    "auth",
    "key",
];

/// Redacts the value of a `NAME=value` assignment whose name is
/// credential-bearing (see [`CREDENTIAL_ENV_PREFIX`] and
/// [`CREDENTIAL_NAME_WORDS`]). The value runs to the next whitespace, `&`, or
/// `;` — the same boundary the `key=value` markers use — so an environment
/// dump line, an `export`, a shell echo, and an inline `x=1; y=2` chain all
/// redact whole values, while ordinary `a=b` comparisons pass through.
pub(super) fn redact_env_assignments(value: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    while let Some(eq) = value[cursor..].find('=') {
        let eq = cursor + eq;
        if !assignment_name(value, eq).is_some_and(is_credential_name) {
            output.push_str(&value[cursor..eq + 1]);
            cursor = eq + 1;
            continue;
        }
        output.push_str(&value[cursor..eq + 1]);
        let start = eq + 1;
        let end = value[start..]
            .find(|c: char| c.is_whitespace() || c == '&' || c == ';')
            .map_or(value.len(), |offset| start + offset);
        output.push_str("[redacted]");
        cursor = end;
    }
    output.push_str(&value[cursor..]);
    output
}

/// The `NAME` of the `NAME=` assignment ending at byte `eq`: the run of
/// ASCII env-name characters immediately before the `=`, never empty.
fn assignment_name(value: &str, eq: usize) -> Option<&str> {
    let bytes = value.as_bytes();
    let start = (0..eq)
        .rev()
        .take_while(|i| matches!(bytes[*i], b'_' | b'-' | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z'))
        .last()
        .unwrap_or(eq);
    (start < eq).then_some(&value[start..eq])
}

/// True when `name` is credential-bearing: in the run's own credential
/// namespace (case-insensitive [`CREDENTIAL_ENV_PREFIX`] prefix), or ending
/// in a [`CREDENTIAL_NAME_WORDS`] word — the whole name, or the word after a
/// `_`/`-` boundary. A trailing `s` is stripped first, so plurals match;
/// without the boundary an ordinary word that merely contains a suffix
/// (`monkey`, `oauth`) never matches.
fn is_credential_name(name: &str) -> bool {
    let prefix = CREDENTIAL_ENV_PREFIX.as_bytes();
    if name.len() >= prefix.len()
        && eq_ignore_ascii_case_slice(&name.as_bytes()[..prefix.len()], prefix)
    {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix('s').unwrap_or(&lower);
    CREDENTIAL_NAME_WORDS.iter().copied().any(|word| {
        stem == word
            || stem.len() > word.len()
                && stem.ends_with(word)
                && matches!(stem.as_bytes()[stem.len() - word.len() - 1], b'_' | b'-')
    })
}
