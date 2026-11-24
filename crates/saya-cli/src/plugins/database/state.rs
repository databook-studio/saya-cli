//! The state a chat run's database plugin carries between turns: the most
//! recent SQL it ran, and the last recall's receipt.
//!
//! Both are turn-scoped facts the hooks write and the executor reads, so they
//! live behind mutexes on the plugin and are read through `&self`. Kept beside
//! the plugin rather than inside its one file so the plugin's own contract —
//! what it activates under and what it advertises — reads without them.

use std::sync::{Arc, Mutex};

use saya_agent::ContextBlock;

use super::DatabasePlugin;
use crate::agent::tools::{CaptureEvent, CaptureHook};
use crate::contracts::RecallReceipt;

impl DatabasePlugin {
    /// Records the SQL the run just executed, so `context()` can carry the
    /// adapt-this-query hint the chat's turn carried. `None` clears it — a turn
    /// that ran no SQL states nothing about one.
    pub(crate) fn record_last_sql(&self, sql: Option<&str>) {
        record(&self.last_sql, sql);
    }

    /// The capture hook the chat surface's tools carry: the front's own hook
    /// (when it drives one) behind the record the next turn's context reads.
    ///
    /// A capture is emitted only for a query that executed — the single
    /// connection arm, on success, either as evidence or refused for its size —
    /// so this is where the run learns the SQL it most recently ran, and the
    /// hint for it rides the NEXT turn's user tail.
    pub(super) fn capture_hook(&self) -> CaptureHook {
        let last_sql = Arc::clone(&self.last_sql);
        let front = self.capture.clone();
        Arc::new(move |event: CaptureEvent| {
            let sql = match &event {
                CaptureEvent::Captured(capture) => Some(capture.sql.as_str()),
                CaptureEvent::Refused { sql, .. } => Some(sql.as_str()),
            };
            record(&last_sql, sql);
            if let Some(front) = &front {
                front(event);
            }
        })
    }

    /// The receipt from the last `context()` call, or `None` when the hook
    /// never recalled — no source attached, or not called yet.
    pub(crate) fn last_receipt(&self) -> Option<Arc<RecallReceipt>> {
        match self.last_receipt.lock() {
            Ok(slot) => slot.clone(),
            // A poisoned lock still holds what the hook wrote: the receipt is a
            // plain value, so read it rather than lose it to an unrelated panic.
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Keeps `receipt` for the executor. One receipt per turn: the newest
    /// recall wins, exactly as the chat's turn reports its own.
    pub(super) fn store_receipt(&self, receipt: Arc<RecallReceipt>) {
        let mut slot = match self.last_receipt.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = Some(receipt);
    }

    /// The last-SQL hint block for this turn, or `None` when the run stated no
    /// SQL — the same block the chat's turn appended, built by the same
    /// function, so the two cannot word it differently.
    pub(super) fn last_sql_block(&self) -> Option<ContextBlock> {
        let sql = match self.last_sql.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        sql.as_deref()
            .and_then(crate::agent::turn_context::last_sql_hint_block)
    }
}

fn record(slot: &Mutex<Option<String>>, sql: Option<&str>) {
    let mut slot = match slot.lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    *slot = sql.map(str::to_owned);
}
