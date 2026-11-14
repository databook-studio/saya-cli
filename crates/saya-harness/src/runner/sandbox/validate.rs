//! The policy-construction guards, split out of `mod.rs` so the policy type
//! stays readable. Every refusal here is fail closed by construction:
//! canonicalise-then-match (measured: Seatbelt resolves paths, so a
//! non-canonical root silently matches nothing), a conservative text class
//! for anything substituted into profile language, and per-platform
//! `net_allow` expressibility — refused at construction, never silently
//! narrowed (entry conditions 1 and 3).

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::SandboxError;

/// Characters a path may contain and still be substituted into Seatbelt
/// profile language. Everything outside the class is refused, never escaped:
/// the escaping semantics of the profile language are unmeasured, and the
/// fail-open alternative was measured (a quoted token that matches
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
/// directory with profile-safe text. Measured: Seatbelt matches
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

/// Resolves one `write_denied` entry against the policy's canonical roots and
/// refuses the shapes the profile language cannot carry.
///
/// The path is NOT canonicalised: the caller may deny a subtree that does not
/// exist yet (`.git` before the first commit). Instead its parent is
/// canonicalised where it exists and the last component is re-joined — the
/// resolved shape is what Seatbelt matches. The result must still start with a
/// canonical root (a denial outside every root names a subtree the policy
/// never allowed anyway), and it must carry the same profile-text class the
/// roots do: it is substituted into profile language.
pub(crate) fn write_denial(raw: &Path, roots: &[PathBuf]) -> Result<PathBuf, SandboxError> {
    let resolved = match (raw.parent(), raw.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
            match fs::canonicalize(parent) {
                Ok(canonical) => canonical.join(name),
                Err(_) => raw.to_path_buf(),
            }
        }
        _ => raw.to_path_buf(),
    };
    if !text_safe(&resolved.to_string_lossy()) {
        return Err(SandboxError::RootNotSafeForProfile {
            path: resolved.display().to_string(),
            reason: format!("path text outside the safe profile class ({ROOT_TEXT_ALLOWED})"),
        });
    }
    if !roots.iter().any(|root| resolved.starts_with(root)) {
        return Err(SandboxError::WriteDenialOutsideRoots {
            path: resolved.display().to_string(),
        });
    }
    Ok(resolved)
}

/// Validates one `net_allow` entry against what this platform's sandbox can
/// express, refusing the rest at construction — never narrowing. macOS: only
/// the loopback hosts (mapped to Seatbelt's `localhost`) and the `*`
/// wildcard are expressible; the host allowlist itself stays with the
/// fetch policy. Linux: nothing is expressible in the netns design
/// [UNVERIFIED — the choice is open]; Windows and every other
/// platform have no sandbox primitive this design targets.
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

/// Maps a policy host onto the Seatbelt host form, or refuses. Measured:
/// the host position accepts ONLY `*` or `localhost` — a raw
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
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::super::RunSandbox;
    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// A canonical scratch root, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "saya-h2-validate-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("the scratch root must be creatable");
            Self(fs::canonicalize(&path).expect("the scratch root must canonicalise"))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A denial outside every root names a subtree the policy never allowed
    /// anyway — a policy bug, refused rather than emitted as a no-op rule.
    #[test]
    fn a_write_denial_outside_every_root_is_refused() {
        let root = Scratch::new();
        let outside = root
            .0
            .parent()
            .expect("the scratch root has a parent")
            .join("saya-h2-not-a-root");
        let error = RunSandbox::new([root.0.clone()], Vec::<(String, u16)>::new())
            .expect("a real root constructs")
            .with_write_denied(outside)
            .expect_err("a denial outside every root is refused");
        assert!(
            matches!(error, SandboxError::WriteDenialOutsideRoots { .. }),
            "the refusal must be typed: {error:?}"
        );
    }

    /// The denial path carries the same profile-text class the roots do: it is
    /// substituted into profile language, so unsafe text is refused, never
    /// escaped.
    #[test]
    fn a_write_denial_with_unsafe_profile_text_is_refused() {
        let root = Scratch::new();
        for name in [
            "quote\"d",
            "new\nline",
            "back\\slash",
            "brace}",
            "open(paren",
            "semi;colon",
        ] {
            let error = RunSandbox::new([root.0.clone()], Vec::<(String, u16)>::new())
                .expect("a real root constructs")
                .with_write_denied(root.0.join(name))
                .expect_err("unsafe profile text is refused");
            assert!(
                matches!(error, SandboxError::RootNotSafeForProfile { .. }),
                "`{name}` must be refused: {error:?}"
            );
        }
    }

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
