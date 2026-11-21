use super::*;
use crate::cli::ReviewDecisionArg;

/// The operation one line translates to. The request's `note` is asserted
/// separately (a canonical spelling states none), so these tests read the
/// operation itself.
fn command_of(line: &str) -> ContractsCommand {
    let rest = line.trim_start_matches('/');
    let (name, arg) = rest.split_once(' ').unwrap_or((rest, ""));
    parse_contract_command(name, arg)
        .unwrap()
        .unwrap_or_else(|| panic!("/{name} is not a contract command"))
        .command
}

#[path = "command_tests/aliases.rs"]
mod aliases;
#[path = "command_tests/approve_all.rs"]
mod approve_all;
#[path = "command_tests/confirm_all.rs"]
mod confirm_all;
#[path = "command_tests/contracts_merge.rs"]
mod contracts_merge;
#[path = "command_tests/decide.rs"]
mod decide;
#[path = "command_tests/forget.rs"]
mod forget;
#[path = "command_tests/queue.rs"]
mod queue;
#[path = "command_tests/unparsed.rs"]
mod unparsed;
#[path = "command_tests/verbs.rs"]
mod verbs;
