//! Twin of the system-prompt rows PT4 deleted with the old assembler
//! (`agent/system_prompt_tests/{answer_contract,assembly,engine_naming}.rs`).
//!
//! The section the chat states is the one copy of the database prompt
//! (`plugins::database::prompt`): the memory briefing, the session's database
//! fact lines, the working guidance, the answer contract, and the engine
//! naming/dialect statement. The old tests read those sentences out of the
//! assembled system prompt; these read the same sentences out of the live
//! section, so the text PT4 kept cannot drift unpinned.
//!
//! This half pins the contract, the memory gate, and the naming rule. The
//! section order, the working guidance, and the combined facts are in
//! `prompt_order_twin_tests.rs`.

use saya_config::MemoryMode;
use saya_types::SqlDialect;

use super::prompt::ANSWER_CONTRACT;
use super::prompt_twin_fixture::section;
use crate::agent::turn_context::memory_reachable;

/// The answer contract's observed-failure clauses ride the live section: the
/// compute directive, the superlative directive, and the scope that leaves
/// non-query answers alone. The bullet count is pinned too, so a clause cannot
/// be dropped by rewriting the list.
#[test]
fn the_answer_contract_keeps_its_observed_failure_clauses() {
    let text = section(&[("local", SqlDialect::Sqlite)], MemoryMode::Off, true);
    for clause in [
        "a ratio, a percentage, or a difference",
        "compute that value and answer with it",
        "returning the operands alone stops one step short",
        "\"the fastest\", \"the highest\", \"the top one\"",
        "\"the fewest\"",
        "asks which one: answer with that row",
        "Every row tied with it is part of the answer",
        "govern answers that report query results",
        "leave other answers untouched",
        "- Return only the columns the question asks for; drop intermediate working columns.",
        "- Do not round unless asked.",
        "- Write dates as ISO YYYY-MM-DD.",
        "- Answer every quantity the question names; if it asks for two things, answer both.",
        "- Read measure words literally: \"volume\" is units, \"revenue\" is money.",
        "- A qualifier on a metric is not a qualifier on the population — filter the metric, not the rows.",
        "- Keep every row tied at a cut-off; never drop a tie to fit a limit.",
        "- When a period is named, enumerate that whole period, not only the rows that happen to appear in the data.",
    ] {
        assert!(
            text.contains(clause),
            "the live answer contract must still state {clause:?}: {text}"
        );
    }
    assert!(
        text.contains(ANSWER_CONTRACT),
        "the contract rides the live section whole: {text}"
    );
    assert_eq!(
        ANSWER_CONTRACT
            .lines()
            .filter(|line| line.starts_with("- "))
            .count(),
        10,
        "no bullet may be added or removed without tripping this row"
    );
}

/// The contract rides every request, so it stays under the ceiling the old
/// test module stated (1200 bytes). The live constant is 1133 bytes.
#[test]
fn the_answer_contract_stays_under_the_stated_ceiling() {
    const ANSWER_CONTRACT_MAX_BYTES: usize = 1200;
    assert!(
        ANSWER_CONTRACT.len() <= ANSWER_CONTRACT_MAX_BYTES,
        "answer contract is {} bytes; the stated ceiling is {ANSWER_CONTRACT_MAX_BYTES}",
        ANSWER_CONTRACT.len(),
    );
}

/// Assisted memory that cannot be honoured states no briefing: the section
/// promises supplied facts and two tools the turn does not have. The gate is
/// the same flag `ChatPrompt::of` reads.
#[test]
fn memory_on_but_unreachable_states_no_briefing() {
    let assisted = section(&[("local", SqlDialect::Sqlite)], MemoryMode::Assisted, true);
    assert!(
        assisted.contains("durable knowledge"),
        "reachable assisted memory states the briefing: {assisted}"
    );
    let unreachable = section(
        &[("local", SqlDialect::Sqlite)],
        MemoryMode::Assisted,
        false,
    );
    assert!(
        !unreachable.contains("durable knowledge"),
        "memory is unreachable, so the briefing must not claim otherwise: {unreachable}"
    );
    assert!(memory_reachable(true, true), "a store and an open gate");
    assert!(
        !memory_reachable(false, true),
        "no state store, no briefing"
    );
    assert!(!memory_reachable(true, false), "privacy gate shut");
}

/// One connection is told the engine and the fullest name its SQL accepts —
/// SQLite must not be asked for `catalog.schema.object`, PostgreSQL must.
#[test]
fn a_single_connection_is_told_its_own_name_form() {
    let sqlite = section(&[("local", SqlDialect::Sqlite)], MemoryMode::Off, true);
    assert!(
        sqlite.contains("The SQL dialect is sqlite's"),
        "the engine is named: {sqlite}"
    );
    assert!(
        sqlite.contains("Name objects as `object`"),
        "SQLite is told the form it accepts: {sqlite}"
    );
    assert!(
        !sqlite.contains("catalog.schema.object"),
        "SQLite cannot parse a three-part name, so the prompt must not ask for one: {sqlite}"
    );

    let postgres = section(&[("main", SqlDialect::Postgres)], MemoryMode::Off, true);
    assert!(
        postgres.contains("The SQL dialect is postgresql's"),
        "the engine is named: {postgres}"
    );
    assert!(
        postgres.contains("Name objects as `catalog.schema.object`"),
        "PostgreSQL accepts the three-part name and is asked for it: {postgres}"
    );
}

/// Every engine the core knows is told its own name form, not another's. The
/// form and the engine's string are read from the same dialect, so a new
/// engine cannot be told the wrong depth.
#[test]
fn every_engine_is_told_the_form_its_sql_accepts() {
    for dialect in [
        SqlDialect::Postgres,
        SqlDialect::Mysql,
        SqlDialect::Snowflake,
        SqlDialect::Sqlite,
        SqlDialect::ClickHouse,
        SqlDialect::BigQuery,
    ] {
        let text = section(&[("main", dialect)], MemoryMode::Off, true);
        assert!(
            text.contains(&format!(
                "Name objects as `{}`",
                dialect.qualified_name_form()
            )),
            "{} must be told its own name form: {text}",
            dialect.as_str()
        );
        assert!(
            text.contains(&format!("The SQL dialect is {}'s", dialect.as_str())),
            "the engine must be named: {text}"
        );
    }
}
