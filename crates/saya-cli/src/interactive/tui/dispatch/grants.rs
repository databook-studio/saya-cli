//! Grant actions: `/allow` seeds the session grant store through the shared
//! behaviour; `/permissions` states the one read-only view (mode, grants,
//! deny list, host lane, data sharing) and `/grants` is its alias.
//! Owns its arms only: the action is taken exactly when this helper owns
//! the arm, and an owned arm always reports handled.

use super::super::transcript::{BlockKind, Transcript};
use super::outcome::Dispatch;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::commands::SessionAction;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;

pub(super) fn apply_grant_action(
    action: &mut Option<SessionAction>,
    transcript: &mut Transcript,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    session: &mut SessionRuntime,
) -> Option<Dispatch> {
    // The arms this helper owns. Anything else is left in `action`
    // untouched for the next helper in the chain.
    if !matches!(
        action,
        Some(SessionAction::Allow(_) | SessionAction::Permissions(_))
    ) {
        return None;
    }
    let current = action.take()?;
    match current {
        SessionAction::Allow(tokens) => {
            // `/allow <scopes…>` seeds the session's one grant store
            // through the shared behaviour — the same parser, the
            // session surface, and the same composition the prompts
            // state: a token the session composed no capability for
            // is refused there too, never seeded. A refused scope is
            // an error and seeds nothing; `/allow none` seeds nothing
            // and says so. Each newly seeded token is journalled once
            // by the shared behaviour; a failed journal write changes
            // no grant and is said in the message.
            match crate::interactive::session::permissions::grants::allow(
                &tokens,
                &session.universe().approval_facts(runtime),
                session.policy().grants(),
                &session.journal(),
            ) {
                Ok(message) => transcript.push(BlockKind::System, message),
                Err(error) => transcript.push(BlockKind::Error, error),
            }
        }
        SessionAction::Permissions(request) => {
            // `/permissions` — one read-only view through the shared
            // `chat::ops_permissions` renderer the headless loop prints too,
            // so the two surfaces cannot state different facts. The mode and
            // the store are the engine's own; the deny list is the runtime's
            // (no probe: the facts the approval prompts read). `/grants`
            // states the same request with its alias spelling, so the note
            // rides the one view.
            let denied = session.universe().approval_facts(runtime).denied_programs;
            transcript.push(
                BlockKind::System,
                crate::chat::ops_permissions::answer(
                    state,
                    request,
                    session.policy().mode(),
                    session.policy().grants(),
                    &denied,
                ),
            );
        }
        // The guard above decided ownership; if it ever drifts, put the
        // action back and report "not mine" rather than dropping it.
        other => {
            *action = Some(other);
            return None;
        }
    }
    Some(Dispatch::Handled)
}
