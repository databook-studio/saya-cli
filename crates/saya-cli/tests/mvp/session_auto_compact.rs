//! Real-binary REPL rows for the automatic-compaction turn boundary: a
//! completed turn whose answering input sits at or above the compact
//! threshold fires EXACTLY ONE summariser request through the same
//! operation manual `/compact` runs; a turn below the threshold and a turn
//! with an unreported numerator never fire.

#[cfg(unix)]
use crate::session_question_budget::harness;

#[cfg(unix)]
const WINDOW: u64 = 1_000_000;

#[cfg(unix)]
fn auto_config() -> String {
    format!(
        "[ai]\ninvestigation_max_elapsed_seconds = 30\ncontext_window_tokens = {WINDOW}\ncompaction = \"auto\"\n"
    )
}

include!("session_auto_compact/threshold_success.rs");
include!("session_auto_compact/threshold_below.rs");
include!("session_auto_compact/missing_usage.rs");
include!("session_auto_compact/failed_latches.rs");
include!("session_auto_compact/manual_rearms.rs");
include!("session_auto_compact/clear_rearms.rs");
