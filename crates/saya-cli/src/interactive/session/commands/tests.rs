use super::*;
use crate::interactive::session::lifecycle::resume::block_on;
use saya_store::{RedactedMessage, RedactedSession, SessionStore};

fn scratch(label: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "saya-session-commands-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("sessions")).unwrap();
    // The tombstone lives in the store root's PARENT (`<root>/../lifecycle`),
    // so scratch roots that are direct children of the temp dir share ONE
    // lifecycle directory across every test in the module — a parallel
    // test's tombstone appeared in another test's "empty" listing. Nesting
    // the sessions root gives each scratch its own parent, and with it its
    // own lifecycle dir.
    base.join("sessions")
}

include!("tests/provider_actions.rs");
include!("tests/usage_history.rs");
include!("tests/mode_run.rs");
include!("tests/session_actions.rs");
include!("tests/session_store.rs");
