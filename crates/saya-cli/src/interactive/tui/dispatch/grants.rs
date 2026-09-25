//! Grant actions: `/allow` seeds the session grant store through the shared
//! behaviour; `/grants` lists it verbatim with the engine's own mode first.

use super::super::transcript::{BlockKind, Transcript};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_commands::SessionAction;
use crate::interactive::session_runtime::SessionRuntime;

pub(super) fn apply_grant_action(
    action: SessionAction,
    transcript: &mut Transcript,
    runtime: &RuntimeConfig,
    session: &mut SessionRuntime,
) -> bool {
    match action {
        SessionAction::Allow(tokens) => {
            // `/allow <scopes…>` seeds the session's one grant store
            // through the shared behaviour — the same parser, the
            // session surface, and the same composition the prompts
            // state: a token the session composed no capability for
            // is refused there too, never seeded. A refused scope is
            // an error and seeds nothing; `/allow none` seeds
            // nothing and says so. Each newly seeded token is
            // journalled once by the shared behaviour; a failed
            // journal write changes no grant and is said in the
            // message.
            match crate::interactive::session_grants::allow(
                &tokens,
                &session.universe().approval_facts(runtime),
                session.policy().grants(),
                &session.journal(),
            ) {
                Ok(message) => transcript.push(BlockKind::System, message),
                Err(error) => transcript.push(BlockKind::Error, error),
            }
        }
        SessionAction::Grants => {
            // `/grants` lists the store verbatim: the words are the
            // record, the same words the prompts offered. The mode
            // is the engine's own — under bypass it is stated first,
            // so the count never reads "nothing runs".
            transcript.push(
                BlockKind::System,
                crate::interactive::session_grants::listing(
                    session.policy().mode(),
                    session.policy().grants(),
                ),
            );
        }
        _ => return false,
    }
    true
}
