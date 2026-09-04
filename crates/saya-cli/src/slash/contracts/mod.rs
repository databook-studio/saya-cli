//! Slash adapters that turn `/contracts`, `/contract`, `/remember`, `/forget`,
//! `/queue`, `/confirm`, `/reject`, and `/approve-all` into the same
//! `ContractsCommand` the headless `saya contracts` parser produces — both
//! paths hand one typed value to the shared `run_contracts` dispatcher.

mod command;
mod remember;

pub(crate) use command::parse_contract_command;
#[allow(unused_imports)]
pub(crate) use remember::{RememberSpec, parse_remember};
