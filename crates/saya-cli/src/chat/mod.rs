//! The chat layer's contract types: what a session tells the surface driving it
//! ([`ChatEvent`]), how one turn ended ([`ChatOutcome`]), what a call puts to
//! the operator ([`Question`]), and the front the session talks to
//! ([`ChatFront`]).
//!
//! Presentation-free: a front decides how any of it looks, so nothing here
//! names a terminal library, a widget, or the renderer.

mod event;
mod front;
mod workspace_shape;

// This `pub(crate)` surface is the chat layer's boundary: the composition, the
// session, the turn, the operator and the fronts above them consume it. Nothing
// in the crate references it yet, so the re-exports read as unused in a lib
// build — they are the boundary, not dead code.
#[allow(unused_imports)]
pub(crate) use event::{Budget, ChatEvent, ChatOutcome, Question};
#[allow(unused_imports)]
pub(crate) use front::ChatFront;
#[allow(unused_imports)]
pub(crate) use workspace_shape::{WorkspaceShape, decide as workspace_shape};

#[cfg(test)]
#[path = "contract_guard_tests.rs"]
mod contract_guard_tests;

#[cfg(test)]
#[path = "workspace_shape_tests.rs"]
mod workspace_shape_tests;
