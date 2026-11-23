//! The one privacy rule the file-facing tools share.
//!
//! A path is private when any of three screens objects to it: the credential
//! and database-export shapes ([`crate::coding::snapshot::sensitive_path`]),
//! the before-image screening tables (`saya_store::screen_private_content`),
//! and the shared redactor — a name it would rewrite is a name the user meant
//! to keep. The search walk refuses private paths as it walks
//! (`search_controlled`), and a `read` of one is refused whole by the same
//! rule, so a mention in the composer cannot reach a file a tool call could
//! not.
//!
//! One implementation: the search scan, the delete predicate and the read
//! tool all call this function, so the three can never drift into three
//! opinions of what "private" means.

/// Whether `path` (workspace-relative, `/`-separated) is one the file tools
/// keep private.
pub fn private_path(path: &str) -> bool {
    crate::coding::snapshot::sensitive_path(path)
        || saya_store::screen_private_content(path, None).is_err()
        || saya_types::redact(path) != path
}

#[cfg(all(test, unix))]
#[path = "private_tests.rs"]
mod tests;
