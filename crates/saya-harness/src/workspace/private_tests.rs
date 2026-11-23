//! The one privacy rule, pinned: the shapes the file tools keep private, and
//! the ordinary names they do not.

use super::private_path;

/// The credential and database-export shapes the file tools refuse, whichever
/// tool asks: a mention in the composer reaches no file a tool call could not.
#[test]
fn credential_and_export_shapes_are_private() {
    for path in [
        ".env",
        ".env.local",
        ".ssh/config",
        ".aws/credentials",
        "id_rsa",
        "id_ed25519",
        "deploy/certs/server.pem",
        "keys/release.key",
        "notes/secrets.toml",
        "data/export.parquet",
        "data/app.db",
    ] {
        assert!(private_path(path), "{path} is one of the private shapes");
    }
}

/// Ordinary source and documentation are content, not credentials: the rule is
/// the shape list, never "hidden" or "contains a word".
#[test]
fn ordinary_paths_are_not_private() {
    for path in [
        "src/main.rs",
        "docs/README.md",
        "tests/certs/README.md",
        "secrets_manager.py",
        ".claude/settings.json",
        ".config/saya.toml",
    ] {
        assert!(!private_path(path), "{path} is ordinary content");
    }
}
