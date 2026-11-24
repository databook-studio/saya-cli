//! The four empty states of `02` §9, row for row, at the spec's two sizes.
//!
//! Every row is asserted verbatim — the words, the indents, the box and the
//! air inside it — because an empty state's whole job is the copy it puts in
//! front of an operator who has nothing else on screen.

use super::empty_fixture::{connected, first_run, no_database, no_workspace, rendered};
use super::empty_rows;

/// `02` §9.1: the first run, before a provider is configured. The box is 56
/// cells wide, centred, with the three keys that get the operator moving.
#[test]
fn the_first_run_reads_as_02_9_1() {
    assert_eq!(
        rendered(&empty_rows(&first_run(), 100, 30)),
        vec![
            "                      ╭──────────────────────────────────────────────────────╮",
            "                      │  Welcome to Saya                                     │",
            "                      │  Saya needs an AI provider before it can answer.     │",
            "                      │                                                      │",
            "                      │  Enter  set up a provider (2 minutes)                │",
            "                      │  d      try the demo database first                  │",
            "                      │  q      quit                                         │",
            "                      ╰──────────────────────────────────────────────────────╯",
        ]
    );
    assert_eq!(
        rendered(&empty_rows(&first_run(), 80, 24)),
        vec![
            "            ╭──────────────────────────────────────────────────────╮",
            "            │  Welcome to Saya                                     │",
            "            │  Saya needs an AI provider before it can answer.     │",
            "            │                                                      │",
            "            │  Enter  set up a provider (2 minutes)                │",
            "            │  d      try the demo database first                  │",
            "            │  q      quit                                         │",
            "            ╰──────────────────────────────────────────────────────╯",
        ]
    );
}

/// `02` §9.2: ready, with no database — the header names the workspace and the
/// absence, the examples are the workspace's, the footer names the way in.
#[test]
fn a_ready_run_without_a_database_reads_as_02_9_2() {
    let rows = vec![
        "  Saya · ~/shop · no database",
        "",
        "  Describe a change or ask about this code. For example:",
        "    › explain how this project is organised",
        "    › describe what changed recently",
        "    › walk me through the main entry point",
        "",
        "  /connect to add a database · ? keys · Ctrl+K commands",
    ];
    assert_eq!(rendered(&empty_rows(&no_database(), 100, 30)), rows);
    assert_eq!(rendered(&empty_rows(&no_database(), 80, 24)), rows);
}

/// `02` §9.3: ready, with the demo database — its dialect and table count in
/// the header, its cached schema's first three questions under the intro.
#[test]
fn a_ready_run_with_a_database_reads_as_02_9_3() {
    let rows = vec![
        "  Saya · ~/shop · demo (sqlite, 12 tables)",
        "",
        "  Ask about demo or describe a change. For example:",
        "    › What are the most common channel values in customer_contacts?",
        "    › What are the most common name values in customers?",
        "    › How does amount_cents change by order_date in orders?",
    ];
    assert_eq!(rendered(&empty_rows(&connected(), 100, 30)), rows);
    assert_eq!(rendered(&empty_rows(&connected(), 80, 24)), rows);
}

/// `02` §9.4: ready, with no workspace bound — the header says so and the one
/// line under it says what is off.
#[test]
fn a_run_without_a_workspace_reads_as_02_9_4() {
    let rows = vec![
        "  Saya · no workspace (outside a git repository) · demo",
        "",
        "  File tools are off here. Start Saya inside a project to edit code.",
    ];
    assert_eq!(rendered(&empty_rows(&no_workspace(), 100, 30)), rows);
    assert_eq!(rendered(&empty_rows(&no_workspace(), 80, 24)), rows);
}
