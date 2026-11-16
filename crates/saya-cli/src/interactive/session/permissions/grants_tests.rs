//! The `/allow` and `/grants` commands: the session grant store's seeding
//! and listing, shared by the headless loop and the TUI dispatch — one
//! operation, two adapters.

use crate::approval_facts::ApprovalFacts;
use crate::grant_token::TurnPrimary;
use crate::grant_token_tests::registry_with_primary;
use crate::interactive::session::permissions::grants::{allow, listing};
use saya_agent::{ApprovalPolicy, SessionGrants, SessionPolicy};
use saya_store::{GrantSource, JournalEvent, SessionJournal};
use std::path::PathBuf;

/// A store with one grant seeded through the engine's own record.
fn store_with(token: &str) -> SessionGrants {
    let grants = SessionGrants::default();
    grants.grant(token);
    grants
}

/// A composition that carries the scopes these tests seed: a bound
/// workspace root and the fetch member — the session shape in which every
/// seeded token below gates something (`/allow` refuses a token the
/// composition cannot carry, so a test that seeds one must compose it).
fn composed_facts() -> ApprovalFacts {
    ApprovalFacts {
        workspace_root: Some(PathBuf::from("/home/user/proj")),
        fetch: Some(crate::approval_facts::FetchFacts {
            fetch_body_bytes: 61_440,
            fetch_seconds: 30,
            fetch_redirects: 5,
            download: None,
        }),
        ..ApprovalFacts::default()
    }
}

/// A fresh state directory per test, the way a session's is created.
fn state_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("saya-allow-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    dir
}

/// A journal whose file cannot be written: the journal path is a directory,
/// so every append fails.
fn broken_journal(label: &str) -> SessionJournal {
    let dir = state_dir(label);
    std::fs::create_dir_all(dir.join("journal.ndjson")).expect("the block is made");
    SessionJournal::open(&dir)
}

include!("grants_tests/listing.rs");

include!("grants_tests/grant_seeding.rs");

include!("grants_tests/grant_refusals.rs");

include!("grants_tests/scope_and_handle.rs");

include!("grants_tests/run_tokens.rs");

include!("grants_tests/composed_commands.rs");

include!("grants_tests/unavailable_capabilities.rs");
