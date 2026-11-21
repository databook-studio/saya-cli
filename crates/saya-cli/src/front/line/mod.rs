//! The line adapter for a chat session: the front the plain REPL and its
//! transcript drive.
//!
//! The session itself is the chat layer's ([`crate::chat`]); this module is only
//! what a line surface adds — the terminal sink, the terminal question, and
//! today's wording for a settled turn.

mod ask;
mod front;
mod signals;
mod turn;

#[allow(unused_imports)]
pub(crate) use front::{LineFront, wording};
#[allow(unused_imports)]
pub(crate) use turn::{SettledTurn, TurnRecord, TurnTotals};

#[cfg(test)]
#[path = "front_tests.rs"]
mod tests;
