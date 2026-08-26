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
/// A readable, filename-safe id: `<UTC stamp>-<nanos hex>`, e.g.
/// `20260826-143210-9f3a`. The nanos suffix keeps ids unique within a second
/// without pulling in a random source; the store's path validation accepts
/// alphanumerics and hyphens.
fn new_id() -> String {
    let now = SystemTime::now();
    new_id_from(now)
}

fn new_id_from(now: SystemTime) -> String {
    let unix = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_seconds = unix.as_secs();
    // Derive UTC calendar fields from the epoch days (civil-from-days).
    let days = i64::try_from(total_seconds / 86_400).unwrap_or(0);
    let (year, month, day) = civil_from_days(days);
    let seconds_today = total_seconds % 86_400;
    let nanos = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.subsec_nanos());
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}-{nanos:04x}",
        year,
        month,
        day,
        seconds_today / 3600,
        (seconds_today % 3600) / 60,
        seconds_today % 60
    )
}

/// Howard Hinnant's `civil_from_days` algorithm: days since 1970-01-01 to
/// (year, month, day) with no external date dependency.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }), m, d)
}

#[cfg(test)]
mod id_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn ids_are_readable_and_filename_safe() {
        let id = new_id_from(SystemTime::UNIX_EPOCH + Duration::from_secs(1_784_000_000));
        // 2026-07-14 era: YYYYMMDD-HHMMSS-xxxx
        assert_eq!(id.len(), 20, "{id}");
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 6);
        assert_eq!(parts[2].len(), 4);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        // Known epoch: 1970-01-01T00:00:10Z renders as the date it is.
        let early = new_id_from(SystemTime::UNIX_EPOCH + Duration::from_secs(10));
        assert!(early.starts_with("19700101-000010-"), "{early}");
    }
}
