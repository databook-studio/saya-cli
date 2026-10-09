//! `--allow <scopes>` parsing: the CLI side of the run approval.
//!
//! A headless run cannot prompt, so its capability set must be stated before
//! anything starts — and it must be stated exactly: an unknown scope name is
//! a usage error, never a silently ignored token (a run that believes it
//! approved more or less than the user typed is a lying run). The grammar
//! mirrors the [`Capabilities`] contract fields; the built approval is
//! exactly the stated scopes, nothing implicit.

use saya_types::{
    Capabilities, Destination, EndpointBindings, FetchScope, InterpreterScope, RunnerScope,
    is_bare_name, is_name_shaped, is_refused_runner_program,
};

include!("scopes/policy.rs");
include!("scopes/parser.rs");
#[cfg(test)]
mod tests {
    use super::*;

    include!("scopes/tests/parse_refusals.rs");
    include!("scopes/tests/grants.rs");
    include!("scopes/tests/grammar.rs");
    include!("scopes/tests/test_helpers.rs");
    include!("scopes/tests/command.rs");
}
