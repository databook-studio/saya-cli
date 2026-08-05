use super::session_state::{SessionLine, SessionState};
use crate::Cli;
use saya_store::{FsSessionStore, SessionStore};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
#[path = "session_resume_tests.rs"]
mod tests;

pub(crate) struct SessionDefaults {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) allow_data_sharing: bool,
    pub(crate) approval_mode: String,
}

pub(crate) fn load_session(
    store: &FsSessionStore,
    cli: &Cli,
    defaults: &SessionDefaults,
) -> Result<SessionState, Box<dyn std::error::Error>> {
    let loaded = if let Some(id) = cli.options.resume.as_deref() {
        block_on(store.load(id))?
    } else if cli.options.continue_session {
        block_on(store.most_recent())?
    } else {
        None
    };
    match loaded {
        Some(value) => Ok(state_from_redacted(value, defaults)),
        None if cli.options.resume.is_some() || cli.options.continue_session => {
            Err("requested session was not found".into())
        }
        None => Ok(SessionState::new(
            new_id(),
            cli.options.profile.clone(),
            defaults.model.clone(),
        )),
    }
}

/// Converts a [`saya_store::RedactedSession`] into a [`SessionState`].
pub(crate) fn state_from_redacted(
    value: saya_store::RedactedSession,
    defaults: &SessionDefaults,
) -> SessionState {
    let profile = value
        .profile
        .clone()
        .or_else(|| value.profile_names.first().cloned());
    let mut state = SessionState::new(value.id, profile, defaults.model.clone());
    state.provider = if value.version < saya_store::SESSION_VERSION || value.provider.is_empty() {
        defaults.provider.clone()
    } else {
        value.provider
    };
    state.allow_data_sharing = if value.version < saya_store::SESSION_VERSION {
        defaults.allow_data_sharing
    } else {
        value.allow_data_sharing
    };
    state.model = if value.version < saya_store::SESSION_VERSION || value.model.is_empty() {
        defaults.model.clone()
    } else {
        value.model
    };
    state.approval_mode =
        if value.version < saya_store::SESSION_VERSION || value.approval_mode.is_empty() {
            defaults.approval_mode.clone()
        } else {
            value.approval_mode
        };
    state.included_profiles = if value.included_profiles.is_empty() {
        value.profile_names.into_iter().skip(1).collect()
    } else {
        value.included_profiles
    };
    state.messages = value
        .messages
        .into_iter()
        .map(|line| SessionLine {
            role: line.role,
            content: line.content,
        })
        .collect();
    state.turns = if value.turns.is_empty() {
        legacy_turns(&state.messages)
    } else {
        value.turns
    };
    state
}

/// Loads a saved session by ID and converts it to a [`SessionState`].
pub(crate) fn resume_session(
    store: &FsSessionStore,
    id: &str,
    defaults: &SessionDefaults,
) -> Result<Option<SessionState>, Box<dyn std::error::Error>> {
    Ok(block_on(store.load(id))?.map(|value| state_from_redacted(value, defaults)))
}

fn legacy_turns(messages: &[SessionLine]) -> Vec<saya_store::RedactedTurn> {
    let safe = messages
        .iter()
        .filter(|message| message.role == "user" || message.role == "assistant")
        .collect::<Vec<_>>();
    safe.chunks_exact(2)
        .filter(|pair| {
            pair[0].role == "user"
                && pair[1].role == "assistant"
                && !pair[1].content.contains("response omitted")
        })
        .map(|pair| saya_store::RedactedTurn {
            user: pair[0].content.clone(),
            assistant: pair[1].content.clone(),
            database_derived: false,
            tools: Vec::new(),
        })
        .collect()
}

pub(crate) fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
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
