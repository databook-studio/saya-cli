//! Hooks: the user's own commands at the agent's lifecycle points.
//!
//! Six events, from the protected top-level `[hooks]` table
//! ([`saya_config::ResolvedHooks`]):
//!
//! - **`pre_tool_use`** runs before a tool call. Exit `2` **blocks** the call
//!   and returns the hook's own bounded, redacted stderr to the model as the
//!   refusal reason; every other exit allows it and reports a diagnostic.
//! - **`post_tool_use`** runs after a call. It observes; it decides nothing.
//! - **`permission_request`** runs only where a call would ask the operator.
//!   Exit `2` denies the ask with the hook's bounded, redacted stderr; exit `0`
//!   is not an approval — the normal prompt still decides.
//! - **`session_start`** runs once when an interactive session or job root
//!   starts; its bounded stdout is offered to the model as context.
//! - **`session_end`** runs best-effort when that session or job root ends,
//!   on a normal end and after a cancel.
//! - **`stop`** runs when the agent finishes a turn. It observes.
//!
//! # A hook can only narrow
//!
//! No hook can grant a tool, widen a scope, change an approval mode, or answer
//! an ask on the user's behalf. The engine is applied *outside* each surface's
//! own executor ([`HookGate`]) and *after* the loop's approval decision, so
//! the only direction it can move a call is from allowed to refused: a hook
//! that exits 0 for a tool the `[tools]` allow list removed, or for a
//! write-shaped tool a plan posture hid, changes nothing — the surface beneath
//! still refuses it. [`HookApprovalGate`] sits in front of the operator's ask
//! for the same reason: it can only refuse an ask that was already going to
//! happen.
//!
//! # One engine, every surface
//!
//! `Hooks::compose` builds one engine per surface from the same resolved
//! table, the same bounds, and the same runner (AGENTS.md: one operation,
//! multiple adapters). Every surface that runs tools is wired: the interactive
//! session and `saya ask` (both through `agent::runtime::run_agent_turn`), a
//! coding job's root (`commands::coding::parent`) and each of its helpers
//! (`agent::tools::child::compose`), and every step of a `saya run`
//! (`commands::run`). A surface with no hook configured, or a mode that keeps
//! no host lane, composes nothing at all.
//!
//! `stop` fires wherever a turn loop ends: the session, `ask`, a coding job's
//! root and each of its helpers, and every step of a `saya run`. A helper's
//! and a run step's loops end inside the run engine, which tells the same hook
//! gate — through [`HookGate`]'s loop-end observer — that its turn finished.
//!
//! `permission_request` is wired where a call can reach an operator: the
//! interactive session and `saya ask`, through [`HookApprovalGate`]. A
//! headless job or run asks nobody, so it has no ask to pre-empt.
//! `session_start` and `session_end` are wired to the two roots that own a
//! session's life: the interactive session ([`SessionHooks`], carried across a
//! recomposition) and a coding job's root.
//!
//! # Bounds
//!
//! Every hook runs through the same [`WorkspaceShell`] a `shell` call runs
//! through — the harness's bounded capture, redaction pass, per-hook timeout,
//! and process-group kill. A hook that hangs is killed and reaped; a hook that
//! fails is one bounded line on stderr. Neither can wedge the agent loop.
//!
//! [`WorkspaceShell`]: saya_harness::runner::WorkspaceShell

mod gate;
mod payload;
mod runner;
mod session;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use gate::{HookApprovalGate, HookGate};
pub(crate) use runner::{HookCaller, Hooks};
pub(crate) use session::SessionHooks;
