//! Slash adapters that turn `/contracts <verb>`, `/remember` and `/forget` —
//! and their one-release alias spellings `/contract`, `/queue`, `/confirm`,
//! `/reject` and `/approve-all` — into the same `ContractsCommand` the headless
//! `saya contracts` parser produces. Both paths hand one typed value to the
//! shared `run_contracts` dispatcher.

mod alias;
mod command;
mod remember;

pub(crate) use alias::ContractsRequest;
pub(crate) use command::parse_contract_command;
#[allow(unused_imports)]
pub(crate) use remember::{RememberSpec, parse_remember};
