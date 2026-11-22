//! The chat layer's contract types: what a session tells the surface driving it
//! ([`ChatEvent`]), how one turn ended ([`ChatOutcome`]), what a call puts to
//! the operator ([`Question`]), and the front the session talks to
//! ([`ChatFront`]), and the operator those calls are put through
//! ([`SessionOperator`]).
//!
//! Presentation-free: a front decides how any of it looks, so nothing here
//! names a terminal library, a widget, or the renderer.

mod compact;
mod compose;
mod compose_plugins;
mod event;
mod front;
mod link;
mod operator;
mod ops;
mod ops_context;
mod ops_continue;
mod ops_job;
pub(crate) mod ops_permissions;
mod ops_rewind;
mod ops_rewind_run;
pub(crate) mod ops_status;
mod ops_status_facts;
mod ops_status_notes;
mod port;
mod queue;
mod rollover;
mod session;
mod session_facts;
mod transcript;
mod turn;
mod workspace_shape;

// This `pub(crate)` surface is the chat layer's boundary: the composition, the
// session, the turn, the operator and the fronts above them consume it. Nothing
// in the crate references it yet, so the re-exports read as unused in a lib
// build — they are the boundary, not dead code.
#[allow(unused_imports)]
pub(crate) use compact::{Compacted, Compaction};
#[allow(unused_imports)]
pub(crate) use compose::{RunComposition, SessionRun};
#[allow(unused_imports)]
pub(crate) use event::{Budget, ChatEvent, ChatOutcome, Question};
#[allow(unused_imports)]
pub(crate) use front::ChatFront;
#[allow(unused_imports)]
pub(crate) use link::{Resume, RunFacts, after_prepare};
#[allow(unused_imports)]
pub(crate) use operator::SessionOperator;
#[allow(unused_imports)]
pub(crate) use ops::{ChatOps, Fate, Op, OpResult};
#[allow(unused_imports)]
pub(crate) use ops_context::{ContextChange, OpsContext, ReadOp, RolloverRequest, SessionOp};
#[allow(unused_imports)]
pub(crate) use ops_continue::{
    CONTINUE_MESSAGE, NOTHING_TO_CONTINUE, Refusal as ContinueRefusal, TURN_RUNNING, paused,
    refusal,
};
#[allow(unused_imports)]
pub(crate) use ops_job::{JobNotes, claim_job_note};
#[allow(unused_imports)]
pub(crate) use ops_rewind::{Checkpoint, Checkpoints, Refusal, rewind, undo};
#[allow(unused_imports)]
pub(crate) use ops_rewind_run::RunCheckpoints;
#[allow(unused_imports)]
pub(crate) use ops_status::{Note, Section, StatusFacts, StatusNotes, StatusPanel, with_note};
#[allow(unused_imports)]
pub(crate) use port::{PortError, Prepared, RunPort};
#[allow(unused_imports)]
pub(crate) use rollover::{Ending, Rollover, RolloverCause, RolloverError};
#[allow(unused_imports)]
pub(crate) use session::{Admission, ChatSession, SessionState};
#[allow(unused_imports)]
pub(crate) use session_facts::SessionFacts;
#[allow(unused_imports)]
pub(crate) use transcript::{SettledTurn, Transcript};
#[allow(unused_imports)]
pub(crate) use turn::{ChatTurn, JobPort};
#[allow(unused_imports)]
pub(crate) use workspace_shape::{Workspace, WorkspaceShape, decide as workspace_shape};

#[cfg(test)]
#[path = "contract_guard_tests.rs"]
mod contract_guard_tests;

#[cfg(test)]
#[path = "operator_front_fixture.rs"]
pub(crate) mod operator_front_fixture;

#[cfg(all(test, unix))]
#[path = "compose_fixture.rs"]
pub(crate) mod compose_fixture;

#[cfg(all(test, unix))]
#[path = "compose_tests.rs"]
mod compose_tests;

#[cfg(all(test, unix))]
#[path = "compose_ask_tests.rs"]
mod compose_ask_tests;

#[cfg(all(test, unix))]
#[path = "compose_mode_tests.rs"]
mod compose_mode_tests;

#[cfg(all(test, unix))]
#[path = "compose_catalog_tests.rs"]
mod compose_catalog_tests;

#[cfg(all(test, unix))]
#[path = "compose_recall_fixture.rs"]
mod compose_recall_fixture;

#[cfg(all(test, unix))]
#[path = "compose_recall_tests.rs"]
mod compose_recall_tests;

#[cfg(all(test, unix))]
#[path = "compose_sql_tests.rs"]
mod compose_sql_tests;

#[cfg(all(test, unix))]
#[path = "compose_origin_tests.rs"]
mod compose_origin_tests;

#[cfg(all(test, unix))]
#[path = "rollover_fixture.rs"]
mod rollover_fixture;

#[cfg(all(test, unix))]
#[path = "rollover_tests.rs"]
mod rollover_tests;

#[cfg(all(test, unix))]
#[path = "rollover_clear_tests.rs"]
mod rollover_clear_tests;

#[cfg(all(test, unix))]
#[path = "compact_provider.rs"]
mod compact_provider;

#[cfg(all(test, unix))]
#[path = "compact_fixture.rs"]
mod compact_fixture;

#[cfg(all(test, unix))]
#[path = "compact_tests.rs"]
mod compact_tests;

#[cfg(all(test, unix))]
#[path = "compact_latch_tests.rs"]
mod compact_latch_tests;

#[cfg(test)]
#[path = "workspace_shape_tests.rs"]
mod workspace_shape_tests;
