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
    AgentEvent, KnowledgeOutcome, ProposedClaimDto, SuppliedClaimDto, SuppliedContractDto,
};
use saya_config::{
    AiProvider, ColorChoice, ConnectionsFile, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedMemory, ThemeChoice,
};
use saya_store::SqliteStateStore;
use saya_types::ClaimStatus;

use super::history::History;
use super::input::InputBuffer;
use super::stream_events::apply_event;
use super::transcript::Transcript;
use super::types::{App, OverlayState, RequestState};
use crate::interactive::session_prompt::StatusView;

/// A minimal `RuntimeConfig` that satisfies the `App` fields `ui::draw` never
/// reads. Built as a struct literal so no config file, env file, or connection
/// file is touched — the only requirement is that the type constructs.
fn unused_runtime() -> Arc<crate::config::runtime::RuntimeConfig> {
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
                context_byte_budget: 256 * 1024,
                show_thinking: false,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
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
            ignored_project_overrides: Vec::new(),
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
fn unused_store() -> SqliteStateStore {
    SqliteStateStore::new(PathBuf::new())
}

/// An idle `App` with an empty transcript and a fixed profile list. Built
/// directly so no history file is read (`App::new` calls `History::load`).
fn empty_app() -> App {
    App {
        sql_task: None,
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
        runtime: unused_runtime(),
        state_db: unused_store(),
        should_quit: false,
    }
}

/// A stable status bar: profile `analytics`, `ollama/qwen`, `read-only`
/// approval, privacy on. The spinner/elapsed fields are not read when the app
/// is idle, so this is the whole status strip.
fn fixed_status() -> StatusView {
    StatusView {
        profile: "analytics".into(),
        included: Vec::new(),
        provider: "ollama".into(),
        model: "qwen".into(),
        approval_mode: "read-only".into(),
        privacy_on: true,
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
fn render_buffer(app: &App, status: &StatusView, w: u16, h: u16) -> String {
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| super::ui::draw(frame, app, status))
        .expect("draw completes");
    format!("{}", terminal.backend())
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

// --- Wide-table horizontal scrolling + copy. ---------------------------------

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
