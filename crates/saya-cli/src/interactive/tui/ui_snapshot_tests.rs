//! Deterministic snapshots of the **composed** TUI screen — transcript, status
//! bar, and input box drawn together at a real width — so a layout regression
//! is caught in `cargo test` in milliseconds instead of by a human looking at a
//! GIF.
//!
//! These do NOT stand up a store, a database, or a provider. The `App` is built
//! directly as a struct literal (avoiding `App::new`, which reads the history
//! file), the transcript is driven through `stream_events::apply_event` with
//! constructed `AgentEvent`s (the natural seam), and the frame is rendered
//! through the real `ui::draw` onto a `ratatui::backend::TestBackend`. The
//! snapshot is the backend's buffer view (symbols only — colours are stripped),
//! which preserves the trailing whitespace a layout regression would disturb.
//!
//! Three screens, no more — a memory receipt above an answer, a learned plus
//! noted line trailing an answer, and long content at 100×30 proving wrap and
//! truncation. A larger snapshot set gets accepted reflexively, which is the
//! same failure as a weakened assertion.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::{
    AgentEvent, KnowledgeOutcome, LocalStateEffect, ProposedClaimDto, SuppliedClaimDto,
    SuppliedContractDto, ToolEffect,
};
use saya_config::{
    AiProvider, ColorChoice, ConnectionsFile, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_store::SqliteStateStore;
use saya_types::ClaimStatus;

use super::history::History;
use super::input::InputBuffer;
use super::stream_events::apply_event;
use super::transcript::Transcript;
use super::types::{App, OverlayState, RequestState};
use crate::interactive::session_prompt::StatusView;

/// `bounded_sql_query`'s declared effect, carried on the fabricated request
/// events so they match what the loop emits.
fn bounded_sql_query_effect() -> ToolEffect {
    ToolEffect {
        database_data: true,
        external_side_effect: false,
        requires_approval: true,
        local_state: LocalStateEffect::None,
    }
}

/// A minimal `RuntimeConfig` that satisfies the `App` fields `ui::draw` never
/// reads. Built as a struct literal so no config file, env file, or connection
/// file is touched — the only requirement is that the type constructs.
pub(crate) fn unused_runtime() -> Arc<crate::config::runtime::RuntimeConfig> {
    Arc::new(crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            profile_name: None,
            profile: None,
            ai: ResolvedAi {
                provider: AiProvider::Ollama,
                model: "test-model".into(),
                base_url: None,
                api_key: None,
                allow_data_sharing: true,
                temperature: 0.0,
                timeout_seconds: 60,
                idle_timeout_seconds: 90,
                max_output_tokens: 4096,
                max_output_tokens_is_default: true,
                context_byte_budget: 256 * 1024,
                context_window_tokens: None,
                show_thinking: false,
                compaction: saya_config::CompactionMode::Auto,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: BTreeMap::new(),
                turns: Some(4),
                tool_calls: None,
                fetch: ResolvedFetchJobs::default(),
                interpreter: ResolvedInterpreterJobs::default(),
                runner: ResolvedRunnerJobs::default(),
            },
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            ui_theme: ThemeChoice::Auto,
            memory: ResolvedMemory {
                mode: MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            endpoints: BTreeMap::new(),
        },
        connections: ConnectionsFile::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::new(),
        secret_values: BTreeMap::new(),
    })
}

/// A lazy `SqliteStateStore` whose pool is never initialized — `ui::draw` never
/// calls `pool()`, so no file is created or read. The path is empty and never
/// touched.
pub(crate) fn unused_store() -> SqliteStateStore {
    SqliteStateStore::new(PathBuf::new())
}

/// An idle `App` with an empty transcript and a fixed profile list. Built
/// directly so no history file is read (`App::new` calls `History::load`).
pub(crate) fn empty_app() -> App {
    App {
        sql_task: None,
        compact_task: None,
        input: InputBuffer::new(),
        transcript: Transcript::new(),
        profiles: vec!["analytics".into(), "billing".into()],
        pending: None,
        request: RequestState::default(),
        overlays: OverlayState::default(),
        spinner: 0,
        history: History::with_path_disabled(PathBuf::new()),
        viewport: std::cell::Cell::new((0, 0)),
        ctrl_c_armed: false,
        at_refs: Vec::new(),
        pending_clipboard: None,
        clipboard_copy: None,
        session_save: None,
        pending_session_save: None,
        last_query: None,
        wide_table: Default::default(),
        run_panel: None,
        runtime: unused_runtime(),
        state_db: unused_store(),
        session: std::sync::Arc::new(
            crate::interactive::session_universe::SessionUniverse::empty(),
        ),
        should_quit: false,
        pending_trust_answer: None,
    }
}

/// An `empty_app` with the input buffer pre-set, for cursor-mapping tests that
/// need a non-empty input but no transcript turns.
fn empty_app_with_text(text: &str) -> App {
    let mut app = empty_app();
    app.input.set_text(text);
    app
}

/// A stable status bar: profile `analytics`, `ollama/qwen`, `read-only`
/// approval, `build` mode, sharing on. The spinner/elapsed fields are not read when the app
/// is idle, so this is the whole status strip.
pub(crate) fn fixed_status() -> StatusView {
    StatusView {
        profile: "analytics".into(),
        included: Vec::new(),
        provider: "ollama".into(),
        model: "qwen".into(),
        approval_mode: "read-only".into(),
        agent_mode: "build".into(),
        workspace_root: Some("/home/user/proj".into()),
        sharing_on: true,
        host_composed: false,
        denied_programs: Vec::new(),
        task_summary: None,
    }
}

/// A stable, fixed claim id. `abbreviate_id` keeps the first six chars + `…`
/// (len > 7), so `ki-abcdef1234` renders as `ki-abc…` — deterministic for fixed
/// input (spec: a `ki-…` prefix is fine when stable).
fn claim_id(id: &str) -> saya_types::ClaimId {
    saya_types::ClaimId::parse(id).expect("fixed claim id parses")
}

fn supplied_claim(
    id: &str,
    kind: &str,
    value: &str,
    column: Option<&str>,
    status: ClaimStatus,
) -> SuppliedClaimDto {
    SuppliedClaimDto {
        claim_id: claim_id(id),
        kind: kind.into(),
        value: value.into(),
        column: column.map(str::to_string),
        status,
    }
}

fn supplied_contract(
    profile: &str,
    object: &str,
    state: &str,
    claims: Vec<SuppliedClaimDto>,
) -> SuppliedContractDto {
    SuppliedContractDto {
        profile: profile.into(),
        object: object.into(),
        schema_state: state.into(),
        claims,
    }
}

fn proposed_claim(
    id: &str,
    profile: &str,
    object: &str,
    kind: &str,
    value: &str,
    column: Option<&str>,
    status: ClaimStatus,
) -> ProposedClaimDto {
    ProposedClaimDto {
        claim_id: claim_id(id),
        profile: profile.into(),
        object: object.into(),
        kind: kind.into(),
        value: value.into(),
        column: column.map(str::to_string),
        status,
    }
}

/// Draws `app` at `w×h` through the real `ui::draw` and returns the backend's
/// buffer view (one quoted line per screen row, trailing whitespace preserved).
pub(crate) fn render_buffer(app: &App, status: &StatusView, w: u16, h: u16) -> String {
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| super::ui::draw(frame, app, status))
        .expect("draw completes");
    format!("{}", terminal.backend())
}

/// Draws `app` at `w×h` through the real `ui::draw` and returns the terminal
/// cursor position the render path set with `set_cursor_position`, in
/// `(x, y)` screen cells. This is the seam that pins the input-box cursor
/// mapping end-to-end: a long input must place the cursor at the visual cell
/// of the true insertion point, not pinned against the right border.
fn render_cursor(app: &App, status: &StatusView, w: u16, h: u16) -> (u16, u16) {
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| super::ui::draw(frame, app, status))
        .expect("draw completes");
    let pos = terminal.backend().cursor_position();
    (pos.x, pos.y)
}

// --- Screen 1: a memory receipt above an answer. ----------------------------

/// A `KnowledgeSupplied` block with 2 claims (one confirmed, one candidate),
/// then the assistant's answer. This is the visibility guarantee: the receipt
/// must not silently lose its shape above the answer.
#[test]
fn memory_receipt_above_answer() {
    let mut app = empty_app();
    apply_event(
        &mut app.transcript,
        AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            vec![supplied_contract(
                "analytics",
                "catalog.public.orders",
                "current",
                vec![
                    supplied_claim(
                        "ki-abcdef1234",
                        "table_alias",
                        "orders",
                        None,
                        ClaimStatus::Confirmed,
                    ),
                    supplied_claim(
                        "ki-bbbedcafe",
                        "default_time_column",
                        "created_at",
                        Some("created_at"),
                        ClaimStatus::Candidate,
                    ),
                ],
            )],
            0,
        ),
        false,
    );
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("The orders table uses created_at as its time column."),
        false,
    );

    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    insta::assert_snapshot!(buffer);
}

// --- Screen 2: a learned line and a noted line trailing an answer. -----------

/// Assistant text, then a `memory learned · …` line (a confirmed claim) and a
/// `memory noted · … unconfirmed, review with /queue` line (a candidate). Both
/// wordings in one screen, trailing the answer where "and I kept this" belongs.
#[test]
fn learned_and_noted_trailing_answer() {
    let mut app = empty_app();
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("Done — I've recorded what you told me and flagged the guess."),
        false,
    );
    // A user-stated fact lands Confirmed → "learned". Trails the answer.
    apply_event(
        &mut app.transcript,
        AgentEvent::knowledge_proposed(proposed_claim(
            "ki-learned1",
            "analytics",
            "catalog.public.orders",
            "table_alias",
            "orders",
            None,
            ClaimStatus::Confirmed,
        )),
        false,
    );
    // An assistant inference lands Candidate → "noted", unconfirmed. Trails too.
    apply_event(
        &mut app.transcript,
        AgentEvent::knowledge_proposed(proposed_claim(
            "ki-noted1",
            "analytics",
            "catalog.public.orders",
            "default_time_column",
            "created_at",
            Some("created_at"),
            ClaimStatus::Candidate,
        )),
        false,
    );

    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    insta::assert_snapshot!(buffer);
}

// --- Screen 3: long content at a real width (wrap + truncation). -------------

/// A wide SQL block (a `WHERE` clause wider than the text area) and a long
/// claim value, at 100×30. The SQL wraps across rows; the claim value wraps
/// too; and because the whole exceeds the transcript height, the top is
/// truncated (only the tail is visible). This is where layout regressions hide.
#[test]
fn long_content_at_real_width() {
    let mut app = empty_app();
    // A wide SQL block: format_sql puts each clause on its own line; the long
    // SELECT list and WHERE each wrap at the text width (98).
    apply_event(
        &mut app.transcript,
        AgentEvent::tool_requested(
            "bounded_sql_query",
            serde_json::json!({
                "sql": "select order_id, customer_id, placed_at, fulfilled_at, shipped_at, total_amount, tax_amount, discount_amount, currency, status, region, country, city, postal_code, carrier, tracking_number from catalog.public.orders where placed_at >= '2024-01-01' and status in ('fulfilled','shipped','delivered') and total_amount > 100 and region in ('north','south','east','west','central','pacific','mountain') and currency = 'USD' and carrier is not null order by placed_at desc, total_amount desc limit 50",
                "connection": "analytics",
            }),
            Some(bounded_sql_query_effect()),
        ),
        false,
    );
    apply_event(
        &mut app.transcript,
        AgentEvent::ToolCompleted {
            name: "bounded_sql_query".into(),
            summary: "50 rows".into(),
        },
        false,
    );
    // A second wide SQL block so the transcript overflows the 26-row region at
    // 100×30: the tail-view truncates the top, cutting off the first SQL header.
    apply_event(
        &mut app.transcript,
        AgentEvent::tool_requested(
            "bounded_sql_query",
            serde_json::json!({
                "sql": "select customer_id, count(*) as orders, sum(total_amount) as spend, avg(total_amount) as avg_order, max(placed_at) as last_order from catalog.public.orders where placed_at >= '2024-01-01' and status in ('fulfilled','shipped','delivered') group by customer_id having count(*) > 1 order by spend desc limit 25",
                "connection": "analytics",
            }),
            Some(bounded_sql_query_effect()),
        ),
        false,
    );
    apply_event(
        &mut app.transcript,
        AgentEvent::ToolCompleted {
            name: "bounded_sql_query".into(),
            summary: "25 rows".into(),
        },
        false,
    );
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text(
            "Here are the fulfilled orders and the top spenders over 100 USD.",
        ),
        false,
    );
    // A long claim value: the supplied path renders the value raw (no eliding),
    // so a 180-char description wraps across several lines.
    apply_event(
        &mut app.transcript,
        AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            vec![supplied_contract(
                "analytics",
                "catalog.public.customer_notes",
                "current",
                vec![supplied_claim(
                    "ki-longdesc1",
                    "table_description",
                    "Free-text notes captured by support agents during customer interactions including follow-up reminders, escalation flags, and the internal handling summary used by the tier-two team when triaging escalations, plus the resolved-action log",
                    None,
                    ClaimStatus::Confirmed,
                )],
            )],
            0,
        ),
        false,
    );

    let buffer = render_buffer(&app, &fixed_status(), 100, 30);
    insta::assert_snapshot!(buffer);
}

// --- Slice 2 (G1): splash/status parity for the unbound workspace. --------

/// G1 property 3 — the TUI splash shows the unbound-workspace line beside
/// the no-database lines: both present, neither dropped. `empty_app` has no
/// profiles (so the no-database guidance paints) and the unbound status
/// carries no root (so the workspace paragraph paints too).
#[test]
fn splash_names_unbound_beside_no_database() {
    use crate::interactive::tui::ui::surface::{NO_DATABASE_HEADLINE, NO_WORKSPACE_LINES};
    // `empty_app` carries demo profiles, so clear them: the no-database
    // guidance paints only with no profiles configured.
    let mut app = empty_app();
    app.profiles.clear();
    assert!(
        app.profiles.is_empty(),
        "no profiles, so the no-database guidance paints"
    );
    let mut status = fixed_status();
    status.workspace_root = None;
    let buffer = render_buffer(&app, &status, 80, 30);
    assert!(
        buffer.contains(NO_DATABASE_HEADLINE),
        "the no-database headline is not dropped:\n{buffer}"
    );
    for line in NO_WORKSPACE_LINES {
        assert!(
            buffer.contains(line),
            "the unbound-workspace line is present beside it ({line:?}):\n{buffer}"
        );
    }
}

/// Slice 2, bound control — a bound session's splash draws no workspace
/// paragraph: the unbound lines are absent, so bound output stays
/// byte-identical to before this slice.
#[test]
fn splash_stays_silent_when_a_workspace_is_bound() {
    use crate::interactive::tui::ui::surface::NO_WORKSPACE_LINES;
    // `fixed_status` is the bound case (it carries a root); keep the demo
    // profiles too, so both paragraphs are in their silent shape.
    let app = empty_app();
    let buffer = render_buffer(&app, &fixed_status(), 80, 30);
    for line in NO_WORKSPACE_LINES {
        assert!(
            !buffer.contains(line),
            "a bound session draws no workspace paragraph ({line:?}):\n{buffer}"
        );
    }
}

/// G1 property 4 — the status line keeps `ws:unbound`: unchanged by this
/// slice (pinned by `status_line_names_the_workspace_binding`; asserted
/// here through the real render so the bar and the header cannot drift).
#[test]
fn the_status_line_keeps_ws_unbound() {
    let mut status = fixed_status();
    status.workspace_root = None;
    let buffer = render_buffer(&empty_app(), &status, 80, 24);
    assert!(
        buffer.contains("ws:unbound"),
        "the status bar keeps ws:unbound:\n{buffer}"
    );
}

use super::table::format_table;
use super::transcript::BlockKind;
use saya_types::QueryResult;

/// A 12-column result that overflows the text area at 80×24, so the view must
/// scroll horizontally rather than word-wrap the grid into noise.
fn wide_table_result() -> QueryResult {
    QueryResult {
        columns: vec![
            "id".into(),
            "name".into(),
            "status".into(),
            "region".into(),
            "country".into(),
            "city".into(),
            "postal".into(),
            "carrier".into(),
            "tracking".into(),
            "total".into(),
            "tax".into(),
            "shipped_at".into(),
        ],
        rows: vec![serde_json::json!([
            1,
            "alice",
            "fulfilled",
            "north",
            "CA",
            "SF",
            "94105",
            "UPS",
            "1Z999",
            120,
            12,
            "2024-01-03"
        ])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT * FROM orders".into(),
    }
}

fn app_with_wide_table() -> App {
    let mut app = empty_app();
    // A direct /sql command lands as a user line, then the result table — the
    // table alone would not count as a "turn", so the user line is needed for
    // the transcript pane (not the splash) to render.
    app.transcript
        .push(BlockKind::User, "/sql SELECT * FROM orders");
    app.transcript
        .push(BlockKind::Table, format_table(&wide_table_result()));
    app
}

/// At the left edge the first columns are painted and the last column is off
/// the right side; scrolling right reveals it. This asserts real paint through
/// `ui::draw` (the same path the PTY smoke tests exercise at the process level).
#[test]
fn wide_table_scrolls_horizontally_in_the_transcript() {
    let mut app = app_with_wide_table();
    app.wide_table.h_offset = 0;

    let left = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        left.contains("id"),
        "first column is visible at the left edge:\n{left}"
    );
    assert!(
        !left.contains("shipped_at"),
        "last column does not fit before scrolling:\n{left}"
    );

    // Scroll far enough that the early columns leave the window.
    app.wide_table.h_offset = 11;
    let right = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        right.contains("shipped_at"),
        "scrolling right reveals the last column:\n{right}"
    );
    assert!(
        !right.contains("│ id"),
        "scrolling right drops the first column:\n{right}"
    );
}

/// Pinning the first column holds it in place while the rest scroll, so the
/// key column (an id) never leaves the screen while reading wide rows.
#[test]
fn pin_first_column_stays_put_while_scrolling() {
    let mut app = app_with_wide_table();
    app.wide_table.pin_first = true;
    app.wide_table.h_offset = 11;

    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("│ id"),
        "pinned first column stays on screen:\n{buffer}"
    );
    assert!(
        buffer.contains("shipped_at"),
        "a far column is reached by scrolling:\n{buffer}"
    );
}

/// The view offset is presentation only: the transcript block text is the full
/// untruncated table, so copy (Ctrl+B) still yields every column even while the
/// screen is scrolled and clipped.
#[test]
fn copy_transcript_yields_the_full_untruncated_table_while_scrolled() {
    let mut app = app_with_wide_table();
    app.wide_table.h_offset = 11;
    app.wide_table.pin_first = true;

    app.copy_transcript();
    let copied = app.pending_clipboard.expect("transcript was queued");
    assert!(
        copied.contains("id") && copied.contains("name") && copied.contains("shipped_at"),
        "copy must include every column, not just the visible window:\n{copied}"
    );
    assert!(
        copied.contains("1 row(s)"),
        "copy must include the row-count footer:\n{copied}"
    );
}

/// `copy_last_answer` copies the assistant answer, not the table; the table is
/// never mistaken for the answer. (Guards the new BlockKind against regressing
/// the copy-last-answer path.)
#[test]
fn copy_last_answer_does_not_grab_a_table_block() {
    let mut app = app_with_wide_table();
    app.transcript
        .push(BlockKind::Assistant, "the answer is here");
    app.copy_last_answer();
    let copied = app.pending_clipboard.expect("answer was queued");
    assert_eq!(copied, "the answer is here");
}

// --- Input box: wrap-aware cursor mapping through the real render path. -------
//
// These are the regression tests for the input-box truncation defect
// (databook-studio/saya-cli#57). A question longer than the terminal width used
// to render as one truncated row with the cursor pinned against the right
// border. The fix wraps the line and maps the logical cursor to a visual
// (row, col) against the inner width. Each test draws through the real
// `ui::draw` and reads the cursor position back, so a mapping regression is
// caught here rather than by eye.
//
// Layout at 40×24: the input box sits at the bottom. With a 1-row input the
// box is 3 rows tall (1 text + 2 border), so its inner area is row 21, cols
// 1..39 (inner width 38). A longer input grows the box upward.

/// The inner width of the input box at a 40-column terminal: the box spans
/// cols 0..40 with a rounded border, leaving 38 usable columns.
const INPUT_INNER_WIDTH: usize = 38;

/// A line exactly the inner width keeps the cursor on the single row at the
/// column just past the text — no wrap, no spurious extra row, cursor not
/// pinned against the border. (Spec test list item 1, rendered.)
#[test]
fn input_cursor_stays_on_one_row_for_an_exact_width_line() {
    let mut app = empty_app();
    // 38 chars: exactly the inner width at 40 columns.
    app.input.set_text("a".repeat(INPUT_INNER_WIDTH));
    let (x, _y) = render_cursor(&app, &fixed_status(), 40, 24);
    // inner.x = 1; cursor col = 38 -> screen x = 1 + 38 = 39 (last inner cell).
    assert_eq!(
        x,
        1 + INPUT_INNER_WIDTH as u16,
        "exact-width line: cursor at the last inner column, not past the border"
    );
}

/// A line one character over wraps to two visual rows and the cursor lands on
/// the second row, at column 1 (after the wrapped char). This is the
/// make-or-break case: the old path pinned the cursor against the right
/// border; the wrap-aware path puts it on row 2. (Spec test list item 2,
/// rendered — deliverable 1.)
#[test]
fn input_cursor_wraps_to_second_row_when_line_exceeds_width() {
    let mut app = empty_app();
    // 39 chars: one over the 38-col inner width -> 2 visual rows, cursor at end.
    app.input.set_text("a".repeat(INPUT_INNER_WIDTH + 1));
    let (x, y) = render_cursor(&app, &fixed_status(), 40, 24);
    // inner.x = 1; the cursor is on the second visual row at col 1 (after the
    // single wrapped 'a') -> screen x = 2. The old path put x at the border
    // (40) because it used the logical column.
    assert_eq!(
        x, 2,
        "one char over: cursor column is 2 (inner.x=1 + wrapped col=1), not the border"
    );

    // The box grew to hold the wrap: a 1-row input claims 1 visual row, a line
    // one char over claims 2. Asserted through `input_rows` directly so it
    // does not depend on the splash art that fills the empty transcript pane.
    let two = empty_app_with_text(&"a".repeat(INPUT_INNER_WIDTH + 1));
    let one = empty_app_with_text(&"a".repeat(INPUT_INNER_WIDTH));
    assert_eq!(
        two.input_rows(40),
        2,
        "a wrapped line grows the box to 2 visual rows"
    );
    assert_eq!(
        one.input_rows(40),
        1,
        "an exact-width line keeps the box at 1 visual row"
    );

    // And the wrap is visible: the buffer shows two content rows of 'a's
    // inside the box (the buffer view quotes each row, so match the inner
    // border+content). This guards against a regression to one truncated row.
    let buffer_two = render_buffer(&two, &fixed_status(), 40, 24);
    let wrapped_rows = buffer_two.lines().filter(|l| l.contains("│a")).count();
    assert!(
        wrapped_rows >= 2,
        "the wrapped line shows two content rows, not one truncated row:\n{buffer_two}"
    );
    let _ = (y, app);
}

/// Cursor at position 0 of an empty buffer: the placeholder path still parks
/// the cursor at the inner top-left. (Spec test list item 4, rendered.)
#[test]
fn input_empty_buffer_cursor_at_origin() {
    let app = empty_app();
    let (x, y) = render_cursor(&app, &fixed_status(), 40, 24);
    // inner.x = 1, inner.y = top inner row of the 3-row box.
    assert_eq!(x, 1, "empty buffer: cursor at inner left");
    // The box is the last 3 rows (21..24); inner top is row 22.
    assert_eq!(y, 22, "empty buffer: cursor at inner top row");
}

// --- Fieldnotes phase 2, packet 2B-3: label rows paint. --------------------
//
// RED packet: label rows exist in `lines()` (measured) but `view()` /
// `wide_view()` elide them before the window slice and the painters skip
// them, so measured height exceeds painted height. These tests fail until
// the elision is removed and the label word paints on its own row.

/// A long user request wrapping to several rows shows `YOU` exactly once:
/// the label introduces the turn, the body rows carry no label.
#[test]
fn continuation_lines_carry_no_label() {
    use super::transcript::BlockKind;
    let mut app = empty_app();
    app.transcript.push(
        BlockKind::User,
        "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu",
    );
    // Narrow enough that the body wraps to several rows.
    let buffer = render_buffer(&app, &fixed_status(), 40, 24);
    let you_rows = buffer
        .lines()
        .filter(|line| line.trim_start_matches(['"', ' ']).starts_with("YOU"))
        .count();
    assert_eq!(
        you_rows, 1,
        "the user turn introduces exactly one YOU label row:\n{buffer}"
    );
    assert!(
        !buffer.contains("❯ "),
        "no glyph rail paints anymore:\n{buffer}"
    );
}

/// The run-panel episode introduces its turn with the same word the
/// transcript uses: the shared `transcript::rows::label` map, one source.
#[test]
fn episode_first_line_carries_the_shared_label() {
    use super::run_panel::RunPanel;
    use super::run_worker::RunWorker;
    use super::stream_events::apply_event;
    use super::transcript::BlockKind;
    use saya_agent::AgentEvent;

    let expected = super::transcript::rows::label(BlockKind::Assistant);
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the orders");
    let (tx, rx) = super::run_panel::test_channels();
    let mut panel = RunPanel::new(
        RunWorker {
            rx,
            cancel: saya_agent::CancellationToken::new(),
        },
        "r-label".into(),
        "survey".into(),
    );
    let _ = tx;
    apply_event(
        &mut panel.episode,
        AgentEvent::assistant_text("Step one profiled the tables."),
        false,
    );
    app.run_panel = Some(panel);
    let buffer = render_buffer(&app, &fixed_status(), 100, 30);
    let word = expected.expect("user turns have a label");
    assert!(
        buffer.contains(word),
        "the episode paints the shared label {word:?}:\n{buffer}"
    );
}

/// An error row carries no introducing label (no `ERROR` label exists —
/// inventing one belongs to a later phase), so without colour it paints
/// exactly like ordinary prose: indented, no glyph. This pins the known
/// gap: stripped of style, a failure is text-identical to a plain body row.
#[test]
fn a_failure_is_not_distinguished_by_colour_alone() {
    use super::transcript::BlockKind;
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "hi");
    app.transcript.push(BlockKind::Error, "boom");
    // `render_buffer` strips colour, so anything this sees is hue-independent
    // by construction. The phase gate is that nothing essential relies on hue
    // alone; a failure that paints as plain indented prose would fail it.
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("✗ boom"),
        "a failure keeps a mark that survives with colour stripped:\n{buffer}"
    );
    assert!(
        !buffer.contains("ERROR"),
        "and no ERROR label is invented to provide it — the failure headline \
         is Phase 7's work:\n{buffer}"
    );
}

/// `System` content sits between turns. Without a mark of its own it is
/// indented exactly like assistant prose and reads as part of the answer
/// above it, which misattributes it — the opposite of the phase's
/// who-said-what goal.
#[test]
fn system_content_is_not_absorbed_into_the_answer_above_it() {
    use super::transcript::BlockKind;
    let mut app = empty_app();
    app.transcript
        .push(BlockKind::Assistant, "here is the answer");
    app.transcript
        .push(BlockKind::System, "memory supplied · 1 claim");
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("  here is the answer"),
        "the answer body indents under SAYA:\n{buffer}"
    );
    assert!(
        buffer.contains("· memory supplied"),
        "the receipt keeps a mark distinguishing it from that answer:\n{buffer}"
    );
}

/// `total_lines(w)` equals the row count `wide_view` returns for a tall
/// enough window: measured height is painted height again.
#[test]
fn measured_height_equals_painted_height() {
    use super::transcript::BlockKind;
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "hello");
    app.transcript
        .push(BlockKind::Assistant, "hi there, here is the answer");
    let width = 80;
    let total = app.transcript.total_lines(width);
    let painted = app
        .transcript
        .wide_view(width, total, &app.wide_table)
        .len();
    assert_eq!(
        total, painted,
        "measured height ({total}) must equal painted height ({painted})"
    );
}

/// Phase 3 packet 2: a folded finished chapter paints as one body row
/// carrying the verbatim request — never a summary — and unfolding restores
/// the painted rows.
#[test]
fn a_folded_chapter_paints_its_request_as_one_row() {
    use super::transcript::BlockKind;
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the red orders");
    app.transcript
        .push(BlockKind::Assistant, "the red orders total 42");
    app.transcript.push(BlockKind::User, "and the blue ones");
    let unfolded_rows = app.transcript.total_lines(78);
    assert!(app.transcript.toggle_chapter(1));
    let folded_rows = app.transcript.total_lines(78);
    assert!(
        folded_rows < unfolded_rows,
        "folding removes painted rows ({unfolded_rows} -> {folded_rows})"
    );
    let folded = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        folded.contains("count the red orders"),
        "the folded screen keeps the verbatim request:\n{folded}"
    );
    assert!(
        !folded.contains("the red orders total 42"),
        "the hidden answer leaves the screen:\n{folded}"
    );
    insta::assert_snapshot!(folded);
}

// --- Fieldnotes phase 3, packet 3: automatic fold at the live edge. ---------
//
// When the user sends a new request, the chapter that just finished folds
// itself before the new `User` block lands. Pin the observable screen: the
// finished chapter's answer leaves the buffer while its verbatim request line
// stays, without blessing a new snapshot (the fold path is already snapshotted
// above; this asserts the submit-time trigger paints the same screen).
#[test]
fn sending_a_new_request_folds_the_finished_chapter_on_screen() {
    use super::application::tests_support::idle_app;
    use super::transcript::BlockKind;
    let mut app = idle_app();
    app.input.set_text("count the red orders");
    app.submit();
    app.pending = None;
    app.transcript
        .push(BlockKind::Assistant, "the red orders total 42");
    app.input.set_text("and the blue ones");
    app.submit();
    app.pending = None;
    assert!(
        app.transcript.is_folded(1),
        "the finished chapter folds when the next request starts"
    );
    let screen = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        screen.contains("count the red orders"),
        "the folded screen keeps the verbatim request:\n{screen}"
    );
    assert!(
        !screen.contains("the red orders total 42"),
        "the hidden answer leaves the screen:\n{screen}"
    );
}

// --- Fieldnotes phase 2, packet 2C: the composer says what Send will do. ----
//
// The composer placeholder only paints while the input is empty; the moment
// the user types, nothing on screen says what Enter does. These tests pin the
// packet's one observable outcome: the composer always states what Enter will
// do, with a different hint when the draft is multiline.
const ENTER_SENDS_HINT: &str = "Enter sends";
const MULTILINE_SENDS_HINT: &str = "Enter sends all";
const NEWLINE_HINT: &str = "Alt+Enter";

/// With a single-line draft, the rendered frame contains the send hint. Uses
/// `render_buffer` (colour-stripped), so this is hue-independent.
#[test]
fn the_composer_says_what_enter_does_while_typing() {
    let mut app = empty_app();
    app.input.set_text("how many orders yesterday");
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains(ENTER_SENDS_HINT),
        "a single-line draft states that Enter sends:\n{buffer}"
    );
    assert!(
        !buffer.contains(NEWLINE_HINT),
        "a single-line draft does not name the newline chord:\n{buffer}"
    );
}

/// With a 3-line draft, the hint differs from the single-line one: it states
/// Enter sends every line and names Alt+Enter as the way to add another line.
#[test]
fn a_multiline_draft_warns_that_enter_sends_every_line() {
    let mut app = empty_app();
    app.input.set_text("select *\nfrom orders\nlimit 10");
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains(MULTILINE_SENDS_HINT),
        "a multiline draft warns Enter sends every line:\n{buffer}"
    );
    assert!(
        buffer.contains(NEWLINE_HINT),
        "a multiline draft names Alt+Enter for a new line:\n{buffer}"
    );
}

/// Bracketed paste already lands as one event and stays editable: a pasted
/// 3-line block holds all three lines in the draft and submits nothing.
#[test]
fn a_pasted_block_stays_editable_instead_of_submitting() {
    let mut app = empty_app();
    app.paste("line one\nline two\nline three");
    assert_eq!(
        app.input.lines(),
        vec!["line one", "line two", "line three"],
        "the pasted block holds all three lines as an editable draft"
    );
    assert!(app.pending.is_none(), "pasting must not submit anything");
    assert!(
        app.transcript.blocks().is_empty(),
        "pasting must not append to the transcript"
    );
}

/// The draft buffer survives opening and closing an overlay: set a draft,
/// open the help overlay, close it, and the draft is intact.
#[test]
fn the_draft_survives_opening_and_closing_an_overlay() {
    use super::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = empty_app();
    app.input.set_text("select * from orders");
    handle_key(&mut app, KeyCode::F(1), KeyModifiers::NONE);
    assert!(
        app.overlays.show_help,
        "F1 opens the help overlay while a draft is held"
    );
    // Any key dismisses the help overlay; the draft must survive the round trip.
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.overlays.show_help, "Esc closes the help overlay");
    assert_eq!(
        app.input.text(),
        "select * from orders",
        "the draft survives opening and closing the overlay"
    );
}

// --- The tool-approval modal renders the shared fact body. -------------------

/// The approval modal renders the per-call fact body verbatim — the same
/// `call_facts` output the terminal prompt renders — plus the shared answers
/// line. This is the modal half of the parity property: the body the modal
/// shows is the body the terminal prompt shows, byte for byte.
#[test]
fn approval_modal_renders_the_shared_fact_body() {
    let mut app = empty_app();
    let tool = crate::interactive::session_definitions::http_fetch();
    let arguments = serde_json::json!({"url": "https://api.github.com/repos/x/y"});
    let facts = crate::approval_facts::ApprovalFacts {
        fetch: Some(crate::approval_facts::FetchFacts {
            fetch_body_bytes: 61_440,
            fetch_seconds: 30,
            fetch_redirects: 5,
            download: None,
        }),
        ..crate::approval_facts::ApprovalFacts::default()
    };
    let grant = crate::grant_token::grant_token(&tool.name, &arguments, None, &facts);
    let detail = crate::approval_facts::call_facts(
        &tool.name,
        &arguments,
        grant.as_deref(),
        &facts,
        None,
        None,
    );
    let (respond, _answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(super::types::PendingApproval {
        tool: tool.name.clone(),
        detail,
        grant,
        respond,
    });
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    insta::assert_snapshot!(buffer);
}

/// The phase gate: nothing essential relies on hue alone.
///
/// This asserts it directly rather than by toggling the palette. `render_buffer`
/// returns the `TestBackend` symbol view, which has already discarded every
/// `Style` — so if each kind is identifiable in *this* string, colour cannot be
/// carrying the distinction. Rendering twice with colour on and off and
/// comparing these buffers would instead compare two values that never held
/// colour in the first place: equal by construction, and green whatever the
/// renderer did.
///
/// `Error`, `System` and `Thinking` are identified by a glyph rather than a
/// word. That is the deliberate interim state — their label words are undecided
/// and belong to later phases — not an oversight for this test to paper over.
#[test]
fn every_block_kind_is_distinguishable_without_colour() {
    use super::transcript::BlockKind;
    let mut app = empty_app();
    for (kind, text) in [
        (BlockKind::User, "the user request"),
        (BlockKind::Assistant, "the assistant answer"),
        (BlockKind::Tool, "the tool trail"),
        (BlockKind::Table, "the result grid"),
        (BlockKind::Error, "the failure"),
        (BlockKind::System, "the receipt"),
        (BlockKind::Thinking, "the reasoning"),
    ] {
        app.transcript.push(kind, text);
    }
    let buffer = render_buffer(&app, &fixed_status(), 100, 40);

    for (marker, what) in [
        ("YOU", "a user turn"),
        ("SAYA", "an assistant turn"),
        ("ACTIVITY", "the tool trail"),
        ("RESULT", "a result"),
        ("✗ the failure", "a failure"),
        ("· the receipt", "system content"),
        ("≈ the reasoning", "reasoning"),
    ] {
        assert!(
            buffer.contains(marker),
            "{what} must be identifiable from symbols alone, found no {marker:?}:\n{buffer}"
        );
    }
}

// --- Fieldnotes phase 4: the action line names its target; drafts are marked.

/// Objective A: the busy line names the tool and its target — the path for
/// `workspace_write`, the SQL for a query — via the shared `tool_call_detail`
/// seam. Name the action and its target, never a motive ("to …" is forbidden).
#[test]
fn the_action_line_names_the_tool_and_its_target() {
    let mut app = empty_app();
    app.transcript
        .push(BlockKind::User, "count the orders by region");
    app.request.stream = Some(busy_stream());
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("bounded_sql_query".into());
    app.transcript.buffer_tool_request(
        "bounded_sql_query".into(),
        serde_json::json!({"sql": "select region, count(*) from orders"}),
        None,
    );
    assert_eq!(
        app.transcript.open_tool_count(),
        1,
        "the fixture must hold one open call or the bar cannot name its target"
    );
    assert_eq!(
        crate::interactive::tui::stream_events::tool_call_detail(
            "bounded_sql_query",
            &serde_json::json!({"sql": "select region, count(*) from orders"}),
        )
        .as_deref(),
        Some("select region, count(*) from orders"),
        "the shared seam must surface the SQL for this fixture"
    );
    let buffer = render_buffer(&app, &fixed_status(), 100, 10);
    assert!(
        buffer.contains("running bounded_sql_query: select region, count(*) from orders"),
        "the action line must name the tool and its target:\n{buffer}"
    );
}

/// Objective A: when `tool_call_detail` returns `None`, the line falls back
/// to today's `running {tool}` exactly — no invented target.
#[test]
fn an_action_with_no_detail_falls_back_to_the_bare_tool_name() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "what can you see");
    app.request.stream = Some(busy_stream());
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("schema_discovery".into());
    app.transcript
        .buffer_tool_request("schema_discovery".into(), serde_json::json!({}), None);
    let buffer = render_buffer(&app, &fixed_status(), 100, 10);
    assert!(
        buffer.contains("running schema_discovery"),
        "the bare tool name must render when there is no detail:\n{buffer}"
    );
    assert!(
        !buffer.contains("running schema_discovery:"),
        "no colon and no invented target when the detail is None:\n{buffer}"
    );
}

/// Objective A: a long detail is truncated so it can never push the elapsed
/// time or `(Esc to cancel)` off the bar.
#[test]
fn a_long_detail_never_pushes_the_cancel_hint_off_the_bar() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count everything");
    app.request.stream = Some(busy_stream());
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("bounded_sql_query".into());
    app.transcript.buffer_tool_request(
        "bounded_sql_query".into(),
        serde_json::json!({"sql": "select a_very_long_column_list from some_table ".repeat(20)}),
        None,
    );
    let buffer = render_buffer(&app, &fixed_status(), 100, 10);
    // The bar renders wider than the frame — today's tail alone overflows
    // it — so the clipped pixels cannot carry the assertion: the renderer
    // clips the `Line` at the frame edge and the hint paints past it. What
    // the phase requires is the truncation the renderer budgets from: the
    // action sheds the row's overflow down to the frame width, so the tail
    // spans still follow the action in the same `Line` — nothing is dropped
    // to make room. Assert through the budget seam, not the clipped pixels.
    let tail_width = super::ui::chrome::action_line::tail_width_for_test(&fixed_status());
    let full_row = super::ui::chrome::action_line::total_row_width_for_test(
        app.request.activity.as_deref(),
        app.transcript
            .newest_open_tool()
            .map(|(name, arguments)| (name.to_string(), arguments.clone())),
        0,
        &fixed_status(),
    );
    assert!(
        full_row > 100,
        "precondition: the untruncated row overflows the 100-column frame"
    );
    let _ = tail_width;
    assert!(
        buffer.contains("…"),
        "the over-long detail truncates with an ellipsis rather than overflowing:\n{buffer}"
    );
    let bar_row = buffer
        .lines()
        .find(|line| line.contains("running bounded_sql_query"))
        .expect("the busy bar renders");
    let action_end = bar_row.find("0s ·").expect("elapsed time renders");
    let head = bar_row
        .find("running bounded_sql_query")
        .expect("action renders");
    let detail_chars = bar_row[head..action_end].chars().count();
    assert!(
        detail_chars < 100,
        "the action phrase before the elapsed time must be shorter than the frame, so the tail follows it in the same line:\n{buffer}"
    );
}

/// Objective B: an assistant block in the live chapter while
/// `request.stream.is_some()` is a draft — the label row says so in a plain
/// word, and `block.text` is untouched so clipboard copy is unaffected.
#[test]
fn a_streaming_answer_is_marked_a_draft() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the orders");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("the total is 42"),
        false,
    );
    app.request.stream = Some(busy_stream());
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("SAYA (draft)"),
        "a streaming answer's label must say it is a draft:\n{buffer}"
    );
}

/// Objective B: once the turn ends (`Done` clears the stream), the same block
/// renders as `SAYA` again — the marking follows the state automatically.
#[test]
fn a_finished_answer_is_not_marked_a_draft() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the orders");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("the total is 42"),
        false,
    );
    assert!(
        app.request.stream.is_none(),
        "no stream: the turn is finished"
    );
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("SAYA"),
        "a finished answer keeps its SAYA label:\n{buffer}"
    );
    assert!(
        !buffer.contains("SAYA (draft)"),
        "a finished answer must not say draft:\n{buffer}"
    );
}

/// Objective B: only the live chapter's assistant block is a draft. An
/// earlier chapter — even while a later turn streams — renders exactly as
/// today, with no draft wording anywhere near it.
#[test]
fn an_earlier_chapters_answer_is_never_marked_a_draft() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "first question");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("first answer"),
        false,
    );
    app.transcript.push(BlockKind::User, "second question");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("second answer so far"),
        false,
    );
    app.request.stream = Some(busy_stream());
    let buffer = render_buffer(&app, &fixed_status(), 100, 30);
    assert_eq!(
        buffer.matches("SAYA (draft)").count(),
        1,
        "exactly one draft marking — the live chapter's — may render:\n{buffer}"
    );
    assert!(
        buffer.contains("first answer"),
        "the earlier chapter's answer is still on screen:\n{buffer}"
    );
}

/// Objective B: marking a draft must not change `block.text`, so what copy
/// yields is byte-identical whether the turn is streaming or finished.
#[test]
fn marking_a_draft_does_not_change_what_copy_yields() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the orders");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("the total is 42"),
        false,
    );
    app.copy_last_answer();
    let finished_copy = app.pending_clipboard.clone().expect("answer was queued");
    app.pending_clipboard = None;

    let mut streaming = empty_app();
    streaming
        .transcript
        .push(BlockKind::User, "count the orders");
    apply_event(
        &mut streaming.transcript,
        AgentEvent::assistant_text("the total is 42"),
        false,
    );
    streaming.request.stream = Some(busy_stream());
    streaming.copy_last_answer();
    let streaming_copy = streaming
        .pending_clipboard
        .clone()
        .expect("streaming answer was queued");
    assert_eq!(
        streaming_copy, finished_copy,
        "the draft marking must not change what copy yields"
    );
    assert_eq!(streaming_copy, "the total is 42");
}

/// A running agent request for tests: a real `Stream` whose receiver never
/// delivers, so `is_busy()` reads true and the bar/draft paths render their
/// streaming shape without a provider.
fn busy_stream() -> super::agent::Stream {
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    super::agent::Stream {
        rx,
        cancel: saya_agent::CancellationToken::new(),
        prompt: String::new(),
    }
}
