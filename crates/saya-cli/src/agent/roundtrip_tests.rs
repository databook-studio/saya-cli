//! Tests for [`super::round_trip`] — each numbered behaviour from the task has
//! its own test, driven by a fake [`Restater`] that records every call in
//! order and returns canned answers. No network, provider, or database is
//! touched.

use super::{Restater, ResultShape, Verdict, round_trip};
use async_trait::async_trait;
use saya_agent::CancellationToken;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// fake
// ---------------------------------------------------------------------------

/// One call to the fake restater, recorded in order so tests can assert the
/// two steps happened in the right sequence.
#[derive(Clone, Debug)]
enum Call {
    Restate {
        sql: String,
        shape: ResultShape,
    },
    Judge {
        question: String,
        restatement: String,
    },
}

/// A scripted `Restater` for tests. Records every call in order, returns
/// canned answers, and can cancel a token mid-round-trip once a given number of
/// calls (restate + judge combined) have completed — so a cancellation test
/// can trip the token between the two steps.
struct ScriptedRestater {
    restate_out: Option<String>,
    judge_out: Option<Verdict>,
    log: Mutex<Vec<Call>>,
    cancel_after: Option<usize>,
    token: Option<CancellationToken>,
    calls: AtomicUsize,
    cancelled: AtomicBool,
}

impl ScriptedRestater {
    fn new() -> Self {
        Self {
            restate_out: None,
            judge_out: None,
            log: Mutex::new(Vec::new()),
            cancel_after: None,
            token: None,
            calls: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
        }
    }

    fn restate_returns(mut self, v: Option<String>) -> Self {
        self.restate_out = v;
        self
    }

    fn judge_returns(mut self, v: Option<Verdict>) -> Self {
        self.judge_out = v;
        self
    }

    /// Cancel the token once this many calls (restate + judge combined) have
    /// completed.
    fn cancel_after(mut self, n: usize, token: CancellationToken) -> Self {
        self.cancel_after = Some(n);
        self.token = Some(token);
        self
    }

    fn log(&self) -> Vec<Call> {
        self.log.lock().unwrap().clone()
    }

    /// Count this call and, if the threshold has been reached, cancel the token
    /// once (so re-entering the threshold on a later call is a no-op).
    fn maybe_cancel(&self) {
        let Some(threshold) = self.cancel_after else {
            return;
        };
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if n < threshold {
            return;
        }
        let Some(token) = &self.token else {
            return;
        };
        if !self.cancelled.swap(true, Ordering::SeqCst) {
            token.cancel();
        }
    }
}

#[async_trait]
impl Restater for ScriptedRestater {
    async fn restate(&self, sql: &str, shape: &ResultShape) -> Option<String> {
        self.log.lock().unwrap().push(Call::Restate {
            sql: sql.to_string(),
            shape: shape.clone(),
        });
        self.maybe_cancel();
        self.restate_out.clone()
    }

    async fn judge(&self, question: &str, restatement: &str) -> Option<Verdict> {
        self.log.lock().unwrap().push(Call::Judge {
            question: question.to_string(),
            restatement: restatement.to_string(),
        });
        self.maybe_cancel();
        self.judge_out.clone()
    }
}

fn shape(columns: &[&str], row_count: usize) -> ResultShape {
    ResultShape {
        row_count,
        columns: columns.iter().map(|s| s.to_string()).collect(),
    }
}

// ===========================================================================
// 1. Blindness is the mechanism — restate is never given the question
// ===========================================================================

include!("roundtrip_tests/restate_and_verdict.rs");

// ===========================================================================
// 5. Cancellation is checked before each model call
// ===========================================================================

include!("roundtrip_tests/cancellation.rs");

// ===========================================================================
// 6. The shape carries no values
// ===========================================================================

include!("roundtrip_tests/privacy_and_prompt_shape.rs");
