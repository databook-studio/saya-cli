//! The rows the saved-session picker lists, as the run the surface is on would
//! read them.
//!
//! Three sources and nothing else, the same three a row is defined by: the
//! store's own listing (newest first, with the stamp its age is measured from),
//! each record's words, and the resume decision this process would reach for it —
//! so the mark a row carries cannot disagree with the resume that follows the
//! `↵`. The decision is the line front's own
//! ([`crate::interactive::session::line_chat::resume_decision`], over the facts
//! it reads for a saved record), never a second reading of the link here.
//!
//! Read off the loop's thread: the store's listing and one load per record are
//! file work, and a slow disk never holds a frame.

use std::time::{SystemTime, UNIX_EPOCH};

use saya_store::{RedactedSession, SessionStore};

use super::ops_deps::OpsDeps;
use super::view::{RowFacts, SessionRow};
use crate::commands::saved_sessions::list_sessions;
use crate::interactive::session::lifecycle::resume_mid;
use crate::interactive::session::line_chat::{record_facts, resume_decision};

/// The rows the picker lists: every saved session the store holds, newest first.
pub(crate) async fn listed(deps: &OpsDeps) -> Result<Vec<SessionRow>, String> {
    let listing = list_sessions(&deps.store)
        .await
        .map_err(|error| error.to_string())?;
    // The lease a record's folder may carry is held under this session's state
    // root, so a row's decision is reached where the run took it.
    let root = crate::commands::coding::root_mode::session_state_root(&deps.runtime.current())
        .map_err(|error| error.to_string())?;
    let now = now_ms();
    let mut rows = Vec::new();
    for stored in &listing.rows {
        // A record that will not load is one the pane cannot offer: it is left
        // out rather than drawn as a row that cannot be resumed.
        let Some(record) = deps.store.load(&stored.id).await.ok().flatten() else {
            continue;
        };
        let facts = record_facts(&root, &record, &deps.state_db).await;
        let decision = resume_decision(&record, facts);
        rows.push(SessionRow::listed(stored, words(&record), &decision, now));
    }
    Ok(rows)
}

/// What a record says about itself: the words an operator knows it by, the
/// database it was on, and how many turns it settled.
fn words(record: &RedactedSession) -> RowFacts {
    RowFacts {
        title: resume_mid::title(record),
        database: record.profile.clone(),
        turns: record.turns.len(),
    }
}

/// The clock a row's age is measured against.
fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default()
}
