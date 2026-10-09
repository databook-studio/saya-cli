//! The agent event stream (`AgentEvent`) the loop emits across a turn. Its
//! constructors live in `builders`; the reasons post-turn learning was skipped
//! in `learning`.

mod agent_event;
mod continuation;

pub use agent_event::AgentEvent;
pub use continuation::ProviderContinuationPhase;

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
