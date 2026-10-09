//! The activation line's property tests (DESIGN test 9): the no-euphemism
//! line, the staged names, the session fork fact, and the none-staged
//! variant — every fact the surfaces must say, byte-pinned here.

#[path = "activation_tests/fork_fact.rs"]
mod fork_fact;
#[path = "activation_tests/grouping.rs"]
mod grouping;
#[path = "activation_tests/journal_consents.rs"]
mod journal_consents;
#[path = "activation_tests/none_staged.rs"]
mod none_staged;
#[path = "activation_tests/probe_refusal.rs"]
mod probe_refusal;
#[path = "activation_tests/staged_line.rs"]
mod staged_line;
