//! The live section's order, working guidance, and combined session facts —
//! the rest of the system-prompt twin (`prompt_twin_tests.rs` pins the
//! contract, the memory gate, and the naming rule).
//!
//! The old assembler stated the connection block before memory. The live
//! section states memory first; this file pins the live order and records the
//! difference in `REPORT.md` rather than asserting an order the live prompt
//! does not have.

use saya_config::MemoryMode;
use saya_types::SqlDialect;

use super::prompt_twin_fixture::section;
use crate::agent::session_facts::workspace_root_text;

/// Two connections are both named, and the live section keeps one order:
/// memory briefing, database fact lines, working guidance, answer contract,
/// engine naming.
#[test]
fn two_connections_are_both_named_in_the_live_section_order() {
    let text = section(
        &[("db1", SqlDialect::Postgres), ("db2", SqlDialect::Sqlite)],
        MemoryMode::Assisted,
        true,
    );
    for line in [
        "- Database `db1` (postgresql).",
        "- Database `db2` (sqlite).",
        "- postgresql: `catalog.schema.object`",
        "- sqlite: `object`",
    ] {
        assert!(text.contains(line), "missing {line:?} in {text}");
    }
    assert!(
        text.contains("The SQL dialect is the connected engine's, whatever the DDL says"),
        "the multi-connection dialect statement rides the section: {text}"
    );
    assert!(
        text.contains("Giving up with a reason is a correct outcome"),
        "the give-up guidance rides the multi-connection section too: {text}"
    );

    let positions = [
        ("SAYA maintains durable knowledge", "memory"),
        ("- Database `db1`", "database fact"),
        ("Multi-step work is expected", "working guidance"),
        ("Answer the question exactly as asked", "answer contract"),
        ("Name objects with the fullest form", "engine naming"),
    ];
    let mut last = 0;
    for (needle, label) in positions {
        let index = text
            .find(needle)
            .unwrap_or_else(|| panic!("the live section must state {label}: {text}"));
        assert!(
            index >= last,
            "{label} moved ahead of the section before it: {text}"
        );
        last = index;
    }
}

/// The working guidance still coaches the turns the old prompt coached:
/// multi-step work, no repeated failed attempt, schema discovery, a missing
/// table or column, asking instead of assuming, and giving up with a reason
/// about the whole session rather than one database.
#[test]
fn the_working_guidance_coaches_multi_step_giving_up_and_clarification() {
    let text = section(&[("local", SqlDialect::Sqlite)], MemoryMode::Off, true);
    for sentence in [
        "Multi-step work is expected",
        "Do not repeat an attempt that already failed",
        "discover the schema before you query it",
        "a missing table or column",
        "call request_clarification instead of assuming",
        "grain",
        "time column",
        "metric",
        "Giving up with a reason is a correct outcome",
        "in this session",
    ] {
        assert!(
            text.contains(sentence),
            "the live guidance must still state {sentence:?}: {text}"
        );
    }
    assert!(
        !text.contains("from this database"),
        "the stopping rule must cover the whole session, not just a database: {text}"
    );
}

/// A run with both databases and a bound root names both fact lines: the
/// section names the connections, the session facts name the root.
#[test]
fn a_run_with_databases_and_a_bound_root_names_both() {
    let text = format!(
        "{}\n\n{}",
        section(
            &[("db1", SqlDialect::Postgres), ("db2", SqlDialect::Sqlite)],
            MemoryMode::Off,
            false,
        ),
        workspace_root_text(std::path::Path::new("/repo")),
    );
    for name in ["db1", "db2", "/repo"] {
        assert!(text.contains(name), "both fact lines name {name}: {text}");
    }
}
