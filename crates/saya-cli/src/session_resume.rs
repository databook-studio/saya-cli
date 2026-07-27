use super::session_state::{SessionLine, SessionState};
use crate::Cli;
use saya_store::{FsSessionStore, SessionStore};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn load_session(
    store: &FsSessionStore,
    cli: &Cli,
) -> Result<SessionState, Box<dyn std::error::Error>> {
    let loaded = if let Some(id) = cli.options.resume.as_deref() {
        block_on(store.load(id))?
    } else if cli.options.continue_session {
        block_on(store.most_recent())?
    } else {
        None
    };
    match loaded {
        Some(value) => {
            let mut state = SessionState::new(
                value.id,
                value.profile_names.first().cloned(),
                "qwen2.5-coder:14b",
            );
            state.included_profiles = value.profile_names.into_iter().skip(1).collect();
            state.messages = value
                .messages
                .into_iter()
                .map(|line| SessionLine {
                    role: line.role,
                    content: line.content,
                })
                .collect();
            Ok(state)
        }
        None if cli.options.resume.is_some() || cli.options.continue_session => {
            Err("requested session was not found".into())
        }
        None => Ok(SessionState::new(
            new_id(),
            cli.options.profile.clone(),
            "qwen2.5-coder:14b",
        )),
    }
}

pub(crate) fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(future)
}
fn new_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().to_string())
        .unwrap_or_else(|_| "session".into())
}
