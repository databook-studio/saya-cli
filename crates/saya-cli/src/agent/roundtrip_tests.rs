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

#[tokio::test]
async fn restate_is_never_given_the_question() {
    // Blindness is enforced two ways: by construction (the question is not a
    // parameter of `restate`) and at runtime (the question text must not reach
    // the restater through the sql or the shape). A restater that can see the
    // question will echo it and the signal disappears.
    let question = "names of high schoolers who have 3 or more friends";
    let sql = "SELECT s.name FROM high_schoolers s JOIN friendships f \
               ON f.student_id = s.id GROUP BY s.id HAVING COUNT(*) >= 3";
    let shape = shape(&["name"], 2);
    let restater = ScriptedRestater::new()
        .restate_returns(Some("which students have at least three friends?".into()))
        .judge_returns(None);

    round_trip(question, sql, &shape, &restater, &CancellationToken::new()).await;

    let log = restater.log();
    // restate was actually called, so blindness can be checked on a real call.
    let Call::Restate {
        sql: seen_sql,
        shape: seen_shape,
    } = &log[0]
    else {
        panic!("first call must be restate, got {:?}", log[0]);
    };
    assert_eq!(
        seen_sql.as_str(),
        sql,
        "restate must receive the SQL verbatim"
    );
    assert_eq!(seen_shape, &shape);
    // The question text must not appear anywhere restate can see it.
    assert!(
        !seen_sql.contains(question),
        "question leaked into the SQL handed to restate"
    );
    assert!(
        seen_shape.columns.iter().all(|c| !c.contains(question)),
        "question leaked into the column names handed to restate"
    );
}

// ===========================================================================
// 2. Two steps, not one — restate first, then judge
// ===========================================================================

#[tokio::test]
async fn two_steps_restate_then_judge_not_one_call() {
    // A single call "does this SQL answer this question?" would hand the model
    // both the SQL and the question at once, reintroducing the blindness
    // problem the split exists to avoid. Here the two calls are made in order,
    // and judge receives the restatement restate produced (not a fresh look at
    // the SQL).
    let question = "how many orders did each customer place?";
    let sql = "SELECT customer_id, COUNT(*) AS n FROM orders GROUP BY customer_id";
    let restatement = "count of orders per customer".to_string();
    let shape = shape(&["customer_id", "n"], 5);
    let restater = ScriptedRestater::new()
        .restate_returns(Some(restatement.clone()))
        .judge_returns(Some(Verdict {
            agrees: true,
            reason: "both count orders per customer".into(),
        }));

    let rt = round_trip(question, sql, &shape, &restater, &CancellationToken::new()).await;

    let log = restater.log();
    assert_eq!(log.len(), 2, "exactly two model calls: restate, then judge");
    assert!(
        matches!(log[0], Call::Restate { .. }),
        "first call must be restate"
    );
    match &log[1] {
        Call::Judge {
            question: q,
            restatement: r,
        } => {
            assert_eq!(
                q, question,
                "judge must receive the question that was asked"
            );
            assert_eq!(
                r, &restatement,
                "judge must receive the restatement restate produced"
            );
        }
        _ => panic!("second call must be judge, got {:?}", log[1]),
    }
    assert_eq!(rt.restatement.as_deref(), Some(restatement.as_str()));
    assert!(rt.verdict.as_ref().is_some_and(|v| v.agrees));
    assert!(!rt.diverged, "an agreeing verdict is not a divergence");
}

// ===========================================================================
// 3. No restatement → no verdict
// ===========================================================================

#[tokio::test]
async fn no_restatement_means_no_verdict_and_no_divergence() {
    // If restate returns None (the model declined or could not restate), judge
    // is not called and the round-trip reports no divergence.
    let question = "how many orders did each customer place?";
    let sql = "SELECT customer_id, COUNT(*) AS n FROM orders GROUP BY customer_id";
    let shape = shape(&["customer_id", "n"], 5);
    let restater = ScriptedRestater::new()
        .restate_returns(None)
        .judge_returns(Some(Verdict {
            agrees: true,
            reason: "judge must not be reached".into(),
        }));

    let rt = round_trip(question, sql, &shape, &restater, &CancellationToken::new()).await;

    let log = restater.log();
    assert_eq!(log.len(), 1, "only restate ran; judge must not be called");
    assert!(matches!(log[0], Call::Restate { .. }));
    assert!(rt.restatement.is_none());
    assert!(rt.verdict.is_none());
    assert!(!rt.diverged, "no restatement can never be a divergence");
}

// ===========================================================================
// 4. A missing verdict is not a divergence
// ===========================================================================

#[tokio::test]
async fn missing_verdict_is_not_a_divergence() {
    // restate produced a restatement, but judge declined (returned None). The
    // restatement is preserved, but absent evidence is never a disagreement.
    let question = "how many orders did each customer place?";
    let sql = "SELECT customer_id, COUNT(*) AS n FROM orders GROUP BY customer_id";
    let restatement = "count of orders per customer".to_string();
    let shape = shape(&["customer_id", "n"], 5);
    let restater = ScriptedRestater::new()
        .restate_returns(Some(restatement.clone()))
        .judge_returns(None);

    let rt = round_trip(question, sql, &shape, &restater, &CancellationToken::new()).await;

    let log = restater.log();
    assert_eq!(log.len(), 2, "both calls ran");
    assert!(matches!(log[1], Call::Judge { .. }));
    assert_eq!(rt.restatement.as_deref(), Some(restatement.as_str()));
    assert!(rt.verdict.is_none());
    assert!(
        !rt.diverged,
        "a missing verdict is absent evidence, never a disagreement"
    );
}

#[tokio::test]
async fn explicit_disagreement_sets_diverged() {
    // The signal itself: only an explicit disagreeing verdict sets diverged.
    // Both the restatement and the verdict are preserved so a reader can see
    // what differed, not just that something did. This is the real miss from
    // our own run — the question asks for *names of students*, the SQL counts
    // *friendship rows per student*.
    let question = "names of high schoolers who have 3 or more friends";
    let sql = "SELECT s.name FROM high_schoolers s JOIN friendships f \
               ON f.student_id = s.id GROUP BY s.id HAVING COUNT(*) >= 3";
    let restatement =
        "students appearing on either side of a friendship at least 3 times".to_string();
    let verdict = Verdict {
        agrees: false,
        reason: "the SQL counts friendships per student, not students per friendship".into(),
    };
    let restater = ScriptedRestater::new()
        .restate_returns(Some(restatement.clone()))
        .judge_returns(Some(verdict.clone()));

    let rt = round_trip(
        question,
        sql,
        &shape(&["name"], 2),
        &restater,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(rt.restatement.as_deref(), Some(restatement.as_str()));
    assert!(!rt.verdict.as_ref().unwrap().agrees);
    assert_eq!(rt.verdict.as_ref().unwrap().reason, verdict.reason);
    assert!(
        rt.diverged,
        "an explicit disagreement is the only thing that sets diverged"
    );
}

// ===========================================================================
// 5. Cancellation is checked before each model call
// ===========================================================================

#[tokio::test]
async fn cancelled_before_restate_returns_no_divergence() {
    // The token is already cancelled, so the check before the first model call
    // returns early. Nothing runs.
    let token = CancellationToken::new();
    token.cancel();
    let restater = ScriptedRestater::new()
        .restate_returns(Some("should not be reached".into()))
        .judge_returns(Some(Verdict {
            agrees: false,
            reason: "should not be reached".into(),
        }));

    let rt = round_trip("q", "SELECT 1 AS a", &shape(&["a"], 0), &restater, &token).await;

    assert!(
        restater.log().is_empty(),
        "no model call should run when cancelled up front"
    );
    assert!(rt.restatement.is_none());
    assert!(rt.verdict.is_none());
    assert!(!rt.diverged);
}

#[tokio::test]
async fn cancelled_before_judge_returns_no_divergence() {
    // restate runs (call 1) and trips the token; the check before judge then
    // returns early. A cancelled round-trip is never a considered one, so it
    // carries no restatement and no verdict even though restate produced one.
    let token = CancellationToken::new();
    let restater = ScriptedRestater::new()
        .restate_returns(Some("count of orders per customer".into()))
        .judge_returns(Some(Verdict {
            agrees: false,
            reason: "should not be reached".into(),
        }))
        .cancel_after(1, token.clone());

    let rt = round_trip(
        "how many orders did each customer place?",
        "SELECT customer_id, COUNT(*) AS n FROM orders GROUP BY customer_id",
        &shape(&["customer_id", "n"], 5),
        &restater,
        &token,
    )
    .await;

    let log = restater.log();
    assert_eq!(log.len(), 1, "restate ran, judge did not");
    assert!(matches!(log[0], Call::Restate { .. }));
    assert!(
        rt.restatement.is_none(),
        "a cancelled round-trip carries no restatement"
    );
    assert!(rt.verdict.is_none());
    assert!(!rt.diverged, "cancellation is never a divergence");
}

// ===========================================================================
// 6. The shape carries no values
// ===========================================================================

#[tokio::test]
async fn restate_receives_only_sql_and_shape_never_values() {
    // The shape carries a row count and column names by construction — no row
    // values. The text handed to restate must be exactly the SQL and that
    // shape: nothing else, and no cell value from the result the shape
    // describes.
    let sql = "SELECT name, grade FROM students WHERE grade = 'A'";
    let shape = shape(&["name", "grade"], 3);
    // A value that would appear in the result rows but must never reach restate.
    let cell_value = "Alice";
    let restater = ScriptedRestater::new()
        .restate_returns(Some("which students got an A?".into()))
        .judge_returns(None);

    round_trip(
        "names and grades of A students",
        sql,
        &shape,
        &restater,
        &CancellationToken::new(),
    )
    .await;

    let log = restater.log();
    // restate was actually called — inspect what it received.
    let Call::Restate {
        sql: seen_sql,
        shape: seen_shape,
    } = &log[0]
    else {
        panic!("first call must be restate, got {:?}", log[0]);
    };
    // Exactly the SQL — nothing appended, no values concatenated.
    assert_eq!(seen_sql.as_str(), sql);
    // Exactly the shape — row count and column names only.
    assert_eq!(seen_shape.row_count, shape.row_count);
    assert_eq!(seen_shape.columns, shape.columns);
    // A cell value never appears in the SQL or any column name.
    assert!(
        !seen_sql.contains(cell_value),
        "cell value leaked into the SQL"
    );
    assert!(
        seen_shape.columns.iter().all(|c| !c.contains(cell_value)),
        "cell value leaked into the column names"
    );
}

// ===========================================================================
// Prompts — the wording requirements, locked
// ===========================================================================

#[test]
fn restate_prompt_asks_for_a_plain_language_question_and_nothing_else() {
    let p = super::prompts::RESTATE_PROMPT;
    assert!(
        p.contains("plain-language question"),
        "must ask for a plain-language question"
    );
    assert!(
        p.to_lowercase().contains("nothing else"),
        "must demand the question and nothing else"
    );
    assert!(p.contains("hedging"), "must forbid hedging");
    // The question must not be part of the restate prompt — blindness by
    // construction extends to the prompt template.
    assert!(
        !p.contains("{question}"),
        "the restate prompt must not reference the question"
    );
}

#[test]
fn judge_prompt_asks_yes_no_one_sentence_and_judges_substance_not_wording() {
    let p = super::prompts::JUDGE_PROMPT;
    assert!(p.contains("YES or NO"), "must ask for a yes/no");
    assert!(
        p.contains("one") && p.contains("sentence"),
        "must ask for one sentence alongside the yes/no"
    );
    assert!(
        p.contains("counted") && p.contains("filtered") && p.contains("grouped"),
        "must call out counting, filtering, and grouping as the substance that matters"
    );
    assert!(
        p.contains("wording"),
        "must say wording differences do not matter"
    );
}
