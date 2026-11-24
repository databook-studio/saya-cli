//! `/resume` on the surface's own line: the record opened in place of the
//! session the surface is on.
//!
//! The line the surface drives is one thread's, and every operation it states
//! waits for the one before it (`super::line_surface`), so a resume typed while
//! a turn runs waits for that turn rather than swapping the run under it — the
//! line front's own rule, kept for free.
//!
//! The record itself is [`crate::interactive::session::lifecycle::resume_mid`]'s:
//! the current session saved first, the record loaded through the store's own
//! resume path, its folder checked, and the refusal in one sentence. What this
//! file adds is what only the surface can do — the run it is on is given up, the
//! record is opened by the one opener a launch uses (`Line::resumed`), the
//! launch's state cell moves with it, and the frame says what happened.

use std::sync::{Arc, Mutex, PoisonError};

use super::msg::ResumedView;
use super::run_front::Forward;
use crate::interactive::session::lifecycle::resume_mid::{self, MidResume, ResumeMid};
use crate::interactive::session::line_chat::Line;

/// How many of a resumed record's turns the transcript shows: enough to read
/// what the conversation was, few enough that an old session does not bury the
/// one the operator just came back to.
pub(crate) const RESUMED_TAIL: usize = 4;

/// What a resume asks the line's thread for: the record's id, the store it is
/// saved in, the session's own state — saved first, replaced when the record
/// opens — and where the run's job records live, which is where a folder's lease
/// is looked for.
pub(crate) struct ResumeAsk {
    pub(crate) id: String,
    pub(crate) store: saya_store::FsSessionStore,
    pub(crate) state: Arc<Mutex<crate::interactive::SessionState>>,
    pub(crate) state_root: std::path::PathBuf,
}

/// The line the surface drives from here on, and the one sentence it says about
/// it — or the refusal, on the line it was on.
pub(crate) async fn resume(line: Line, front: &Forward, ask: ResumeAsk) -> Line {
    let current = ask
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let loaded = resume_mid::resume_mid_session(ResumeMid {
        store: &ask.store,
        current: &current,
        id: &ask.id,
        state_root: &ask.state_root,
    })
    .await;
    let loaded = match loaded {
        MidResume::Loaded(loaded) => loaded,
        MidResume::Refused(sentence) => {
            front.say(sentence);
            return line;
        }
    };
    let record = resume_mid::record_of(&loaded);
    let notice = resume_mid::resumed_notice(&record);
    let turns = tail(&record);
    let model = loaded.model.clone();
    let database = loaded.profile.clone();
    let (line, opened) = line.resumed(&loaded).await;
    match opened {
        Ok(said) => {
            // The session the surface is on is the record's from here on: the
            // launch's own state cell moves with it, so the next save writes the
            // resumed session and the row reads the facts it states.
            *ask.state.lock().unwrap_or_else(PoisonError::into_inner) = *loaded;
            if let Some(said) = said {
                front.note(said);
            }
            front.resumed(ResumedView {
                notice,
                turns,
                model,
                database,
            });
            line
        }
        Err(sentence) => {
            front.say(sentence);
            line
        }
    }
}

/// The turns the transcript shows for a resumed record: its last few, oldest
/// first, so the operator reads the conversation they came back to without the
/// pane filling with an old one.
fn tail(record: &saya_store::RedactedSession) -> Vec<(String, String)> {
    let turns = &record.turns;
    let from = turns.len().saturating_sub(RESUMED_TAIL);
    turns[from..]
        .iter()
        .map(|turn| (turn.user.clone(), turn.assistant.clone()))
        .collect()
}
