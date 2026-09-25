//! The policy-construction guards, split out of `mod.rs` so the policy type
//! stays readable. Every refusal here is fail closed by construction:
//! canonicalise-then-match (measured: Seatbelt resolves paths, so a
//! non-canonical root silently matches nothing), a conservative text class
//! for anything substituted into profile language, and per-platform
//! `net_allow` expressibility — refused at construction, never silently
//! narrowed (spike §8, entry conditions 1 and 3).

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::SandboxError;

/// Characters a path may contain and still be substituted into Seatbelt
/// profile language. Everything outside the class is refused, never escaped:
/// the escaping semantics of the profile language are unmeasured, and the
/// spike measured the fail-open alternative (a quoted token that matches
/// nothing parses fine and silently allows nothing — while its *complement*,
/// a deny rule quoting attacker text, would silently deny nothing it was
/// meant to deny). `"` and `\` end or alter a string literal; `{` `}` are
/// the generator's own placeholder markers; `(` `)` open and close
/// expressions; controls and non-ASCII are unmeasured in string literals.
const ROOT_TEXT_ALLOWED: &str = "ASCII printable, minus \" \\ { } ( ) ;";

fn root_text_safe(path: &Path) -> bool {
    text_safe(&path.to_string_lossy())
}

/// The safe profile-text class, as a function over plain text: every
/// substitution into profile language — roots, `net_allow` hosts, and the
/// `process-fork` reason comment — must carry it.
pub(crate) fn text_safe(text: &str) -> bool {
    text.is_ascii()
        && text.chars().all(|c| {
            (' '..='~').contains(&c) && !matches!(c, '"' | '\\' | '{' | '}' | '(' | ')' | ';')
        })
}

/// Canonicalises one `fs_roots` entry and refuses it unless it is a
/// directory with profile-safe text. Measured (spike §4.2): Seatbelt matches
/// the resolved path, so the canonical form is the only form that can be
/// substituted; a root that cannot be canonicalised refuses the policy
/// rather than degrading to a profile that allows nothing or, worse, one
/// whose denials happen to still cover the unresolved shape.
pub(crate) fn canonical_root(raw: &Path) -> Result<PathBuf, SandboxError> {
    if raw.as_os_str().is_empty() {
        return Err(SandboxError::RootNotSafeForProfile {
            path: raw.display().to_string(),
            reason: "empty path".into(),
        });
    }
    let canonical = fs::canonicalize(raw).map_err(|source| SandboxError::Io {
        context: format!("canonicalise sandbox fs root {}", raw.display()),
        source,
    })?;
    if !canonical.is_dir() {
        return Err(SandboxError::RootNotSafeForProfile {
            path: canonical.display().to_string(),
            reason: "not a directory".into(),
        });
    }
    // A `subpath "/"` rule would void the whole filesystem confinement, so
    // the filesystem root itself is refused as a root, at construction.
    if canonical.parent().is_none() {
        return Err(SandboxError::RootNotSafeForProfile {
            path: canonical.display().to_string(),
            reason: "the filesystem root cannot be a sandbox root".into(),
        });
    }
    if !root_text_safe(&canonical) {
        return Err(SandboxError::RootNotSafeForProfile {
            path: canonical.display().to_string(),
            reason: format!("path text outside the safe profile class ({ROOT_TEXT_ALLOWED})"),
        });
    }
    Ok(canonical)
}

/// Validates one `net_allow` entry against what this platform's sandbox can
/// express, refusing the rest at construction — never narrowing. macOS: only
/// the loopback hosts (mapped to Seatbelt's `localhost`) and the `*`
/// wildcard are expressible; the host allowlist itself stays with the M3-1
/// fetch policy. Linux: nothing is expressible in the netns design
/// [UNVERIFIED — draft §4 leaves the choice open]; Windows and every other
/// platform have no sandbox primitive this design targets (U9).
pub(crate) fn net_entry(host: &str, port: u16) -> Result<(), SandboxError> {
    if port == 0 {
        return Err(SandboxError::NetPortInvalid { port });
    }
    if cfg!(target_os = "macos") {
        return match seatbelt_host(host) {
            Some(_) => Ok(()),
            None => Err(SandboxError::NetHostNotExpressible {
                host: host.to_string(),
                platform: "macos",
                reason: "Seatbelt accepts only * or localhost in the remote filter; the \
                         host allowlist is enforced in-process by the M3-1 fetch policy",
            }),
        };
    }
    Err(SandboxError::NetHostNotExpressible {
        host: host.to_string(),
        platform: std::env::consts::OS,
        reason: if cfg!(target_os = "linux") {
            "the netns design denies all egress and Landlock's port rules have no host \
             dimension; the Linux egress shape is an unmeasured reviewer decision (draft §4)"
        } else {
            "no OS sandbox primitive this design targets (U9); fail closed"
        },
    })
}

/// Maps a policy host onto the Seatbelt host form, or refuses. Measured
/// (spike §4.3): the host position accepts ONLY `*` or `localhost` — a raw
/// IP or a DNS name is rejected at parse — so the policy's loopback hosts map
/// to `localhost`, and nothing else is expressible. `None` means the policy
/// host cannot be honoured by this profile language and must be refused at
/// construction, never silently narrowed.
pub(crate) fn seatbelt_host(policy_host: &str) -> Option<&'static str> {
    match policy_host {
        "*" => Some("*"),
        h if h.eq_ignore_ascii_case("localhost") => Some("localhost"),
        "127.0.0.1" | "::1" => Some("localhost"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seatbelt_host_maps_only_loopback_and_wildcard() {
        assert_eq!(seatbelt_host("localhost"), Some("localhost"));
        assert_eq!(seatbelt_host("LOCALHOST"), Some("localhost"));
        assert_eq!(seatbelt_host("127.0.0.1"), Some("localhost"));
        assert_eq!(seatbelt_host("::1"), Some("localhost"));
        assert_eq!(seatbelt_host("*"), Some("*"));
        assert_eq!(seatbelt_host("example.com"), None);
        assert_eq!(seatbelt_host("10.0.0.1"), None);
        assert_eq!(seatbelt_host("0.0.0.0"), None);
        assert_eq!(seatbelt_host(""), None);
    }

    #[test]
    fn root_text_class_refuses_profile_language() {
        for text in [
            "/tmp/ok-root-name",
            "/tmp/ok with spaces",
            "/tmp/ok.dot-dash_~",
        ] {
            assert!(root_text_safe(Path::new(text)), "{text} must pass");
        }
        for text in [
            "/tmp/quote\"root",
            "/tmp/back\\slash",
            "/tmp/close)root",
            "/tmp/open(root",
            "/tmp/brace}root",
            "/tmp/{placeholder}",
            "/tmp/semi;colon",
            "/tmp/control\nroot",
            "/tmp/emoji-🦆",
        ] {
            assert!(!root_text_safe(Path::new(text)), "{text} must be refused");
        }
    }
}
