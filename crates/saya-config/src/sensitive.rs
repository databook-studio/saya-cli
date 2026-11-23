//! The sensitive-path set: the one table that says a path holds a credential
//! shape.
//!
//! One table, three readers: the snapshot omits these paths (and reports a
//! tracked one as a warning), the workspace's search and delete predicates
//! refuse them, and the vetted read-only shell set refuses to run a command
//! that names one. A second list would let the shell read what the tools will
//! not write, search, or delete, so there is only this one.

/// One rule in the sensitive-path set.
#[derive(Clone, Copy)]
enum SensitiveRule {
    /// A directory component whose contents are always private.
    Component(&'static str),
    /// A file name that is always private.
    Exact(&'static str),
    /// A file-name prefix (the `.env.*` family).
    Prefix(&'static str),
    /// A file extension, without the dot.
    Extension(&'static str),
}

/// The complete sensitive-path set, shared by snapshot selection and the
/// in-place workspace predicates (delete, search, before-images). Exact
/// credential names, the `.env.*` family, the key/certificate extensions, and
/// the credential-store directories. Names that merely contain `secret` or
/// `credentials` (for example `secrets_manager.py`) and generic `.config`,
/// `.claude`, or `.docker` directories are ordinary repository content and are
/// deliberately absent.
const SENSITIVE_PATHS: &[SensitiveRule] = &[
    SensitiveRule::Component(".aws"),
    SensitiveRule::Component(".ssh"),
    SensitiveRule::Component(".azure"),
    SensitiveRule::Component(".gnupg"),
    SensitiveRule::Component(".kube"),
    SensitiveRule::Component(".codex"),
    SensitiveRule::Component(".saya"),
    SensitiveRule::Exact(".env"),
    SensitiveRule::Exact(".netrc"),
    SensitiveRule::Exact(".npmrc"),
    SensitiveRule::Exact(".pypirc"),
    SensitiveRule::Exact("credentials.json"),
    SensitiveRule::Exact("id_rsa"),
    SensitiveRule::Exact("id_ed25519"),
    SensitiveRule::Prefix(".env."),
    SensitiveRule::Extension("pem"),
    SensitiveRule::Extension("key"),
    SensitiveRule::Extension("p12"),
    SensitiveRule::Extension("pfx"),
];

/// Whether a path holds a credential shape. The one rule table above decides
/// both snapshot omission and the in-place workspace predicates; a tracked
/// path is repository content and is included with a warning instead of being
/// omitted.
pub fn sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    SENSITIVE_PATHS.iter().any(|rule| match *rule {
        SensitiveRule::Component(component) => lower.split('/').any(|part| part == component),
        SensitiveRule::Exact(exact) => name == exact,
        SensitiveRule::Prefix(prefix) => name.starts_with(prefix),
        SensitiveRule::Extension(extension) => name
            .rsplit_once('.')
            .is_some_and(|(_, found)| found == extension),
    })
}
