//! The capability scopes a run declares and a plan's steps must stay inside.
//!
//! The run's `Capabilities` are the approval surface for the whole run: what
//! the user (or a headless `--allow`) approved once, at `planned → approved`.
//! A plan's steps are validated against it — a step asking for a scope the
//! run was not approved for is rejected, never silently granted.

mod capabilities;
mod destination;
mod endpoints;
mod interpreter;
mod naming;
mod runner;

pub use capabilities::Capabilities;
pub use destination::{Destination, FetchScope, MAX_FETCH_DESTINATIONS};
pub use endpoints::{EndpointBindings, MAX_ENDPOINT_BINDINGS};
pub use interpreter::InterpreterScope;
pub use naming::{is_bare_name, is_name_shaped};
pub use runner::{MAX_RUNNER_PROGRAMS, RunnerScope, is_refused_runner_program};
