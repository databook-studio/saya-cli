use super::{SessionDefaults, load_session};
use crate::{Cli, GlobalOptions};
use saya_store::{FsSessionStore, RedactedMessage, RedactedSession, SessionStore};

fn cli() -> Cli {
    Cli {
        options: GlobalOptions {
            continue_session: true,
            ..Default::default()
        },
        command: None,
    }
}

include!("session_resume_tests/defaults_and_migration.rs");

include!("session_resume_tests/mode_and_grants.rs");

include!("session_resume_tests/task_list.rs");

include!("session_resume_tests/legacy_fields.rs");

include!("session_resume_tests/bypass_and_fallback.rs");
