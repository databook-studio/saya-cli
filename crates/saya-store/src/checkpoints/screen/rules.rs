//! The path tables of admission screening: which workspace-relative paths
//! look like credential files, and which look like database-export
//! artifacts. Case-insensitive; the path splits on both `/` and `\`.
//! Deliberately narrower than the release-packaging denylist: checkpoints
//! cover a user's ordinary source edits, so tooling and configuration
//! directories are legitimate edit targets and are NOT refused here.

const CREDENTIAL_DIRS: [&str; 6] = [".aws", ".ssh", ".gnupg", ".kube", ".docker", ".saya"];

const CREDENTIAL_NAMES: [&str; 16] = [
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    ".netrc",
    ".pgpass",
    ".npmrc",
    ".pypirc",
    "credentials",
    "credentials.json",
    "secrets",
    "secrets.toml",
    "secret.json",
    "token.json",
    "auth.json",
    "connections.toml",
];

/// Key-material extensions: the extension alone refuses the final
/// component.
const KEY_EXTENSIONS: [&str; 8] = ["pem", "key", "p8", "p12", "pfx", "keystore", "jks", "kdbx"];

/// Extensions for which a credential-named stem refuses the final
/// component. `.env.example` is refused too — conservative by design.
const STEM_EXTENSIONS: [&str; 9] = [
    "json", "toml", "yaml", "yml", "ini", "txt", "env", "conf", "cfg",
];

/// Stem words naming credential material, matched at `.`/`_`/`-`
/// boundaries — the hand-rolled form of
/// `(^|[._-])(credential|credentials|secret|secrets|token|auth)($|[._-])`
/// (the `regex` crate is not a dependency).
const STEM_WORDS: [&str; 6] = [
    "credential",
    "credentials",
    "secret",
    "secrets",
    "token",
    "auth",
];

const EXPORT_EXTENSIONS: [&str; 5] = ["parquet", "duckdb", "sqlite", "sqlite3", "db"];
const CHART_EXPORT_PREFIX: &str = "saya-chart-";

/// True when any path component is a credential directory, a credential
/// file name, or `.env*`, or when the final component carries a
/// key-material extension or a credential-named stem under a
/// text/config extension.
pub(super) fn refuses_credential_path(path: &str) -> bool {
    let mut refused = false;
    let mut final_component = "";
    for component in path.split(['/', '\\']) {
        let lower = component.to_ascii_lowercase();
        if CREDENTIAL_DIRS.contains(&lower.as_str())
            || CREDENTIAL_NAMES.contains(&lower.as_str())
            || lower.starts_with(".env")
        {
            refused = true;
        }
        final_component = component;
    }
    refused || refuses_final_component(final_component)
}

/// True when the final component names key material by extension, or names
/// credentials in its stem while wearing a text/config extension.
fn refuses_final_component(component: &str) -> bool {
    let lower = component.to_ascii_lowercase();
    let Some((stem, extension)) = lower.rsplit_once('.') else {
        return false;
    };
    KEY_EXTENSIONS.contains(&extension)
        || (STEM_EXTENSIONS.contains(&extension)
            && stem
                .split(['.', '_', '-'])
                .any(|word| STEM_WORDS.contains(&word)))
}

/// True when the final component looks like a database-export artifact: an
/// export extension, the staged `source.duckdb` name, or a
/// `saya-chart-*.html` chart export.
pub(super) fn is_database_export_path(path: &str) -> bool {
    let Some(final_component) = path.split(['/', '\\']).next_back() else {
        return false;
    };
    let lower = final_component.to_ascii_lowercase();
    let Some((_, extension)) = lower.rsplit_once('.') else {
        return false;
    };
    EXPORT_EXTENSIONS.contains(&extension)
        || lower == "source.duckdb"
        || (extension == "html" && lower.starts_with(CHART_EXPORT_PREFIX))
}
