//! Provider-reported token usage for a job, read from its durable transport
//! spans. Only reported numbers are summed; a missing breakdown stays absent.
//! Completeness is measured on the input/output totals every span reports, and
//! an unreported span is named rather than estimated.

use super::{CodingApplication, CodingError};
use saya_types::{JobId, TransportOutcome, TransportSpanData};
use serde_json::{Value, json};

/// The store serves at most 100 attempts per page. Sixty-four pages cover the
/// default `transport_attempts` backstop (6,000) with room to spare; a larger
/// job reports what fit and says so instead of reading without a bound.
const PAGE: usize = 100;
const MAX_PAGES: usize = 64;

#[derive(Default)]
struct Totals {
    requests: u64,
    input: u64,
    cached_input: Option<u64>,
    output: u64,
    reasoning: Option<u64>,
    cost_micros: Option<u64>,
    wall_time_ns: u64,
    missing_input: u64,
    missing_output: u64,
}

impl Totals {
    fn add(&mut self, span: &TransportSpanData) {
        // A never-dispatched attempt consumed no tokens, so it is no gap.
        if span.outcome == TransportOutcome::NotStarted {
            return;
        }
        if let Some(start) = span.dispatch_started_ns {
            self.wall_time_ns = self
                .wall_time_ns
                .saturating_add(span.finished_ns.saturating_sub(start));
        }
        let (Some(input), Some(output)) = (span.reported_input_tokens, span.reported_output_tokens)
        else {
            self.missing_input += u64::from(span.reported_input_tokens.is_none());
            self.missing_output += u64::from(span.reported_output_tokens.is_none());
            return;
        };
        self.requests += 1;
        self.input = self.input.saturating_add(input);
        self.output = self.output.saturating_add(output);
        if let Some(cached) = span.reported_cached_input_tokens {
            self.cached_input = Some(self.cached_input.unwrap_or(0).saturating_add(cached));
        }
        if let Some(reasoning) = span.reported_reasoning_tokens {
            self.reasoning = Some(self.reasoning.unwrap_or(0).saturating_add(reasoning));
        }
        if let Some(cost) = span.reported_cost_micros {
            self.cost_micros = Some(self.cost_micros.unwrap_or(0).saturating_add(cost));
        }
    }

    fn view(self, read_truncated: bool) -> Value {
        let complete = self.missing_input == 0 && self.missing_output == 0 && !read_truncated;
        let mut usage = json!({
            "source": "durable_transport_spans",
            "complete": complete,
            "requests": self.requests,
            "input": self.input,
            "cached_input": self.cached_input,
            "output": self.output,
            "model_wall_time_ms": self.wall_time_ns / 1_000_000,
        });
        if let Some(reasoning) = self.reasoning {
            usage["reasoning"] = json!(reasoning);
        }
        if let Some(cost) = self.cost_micros {
            usage["cost_usd"] = json!(cost as f64 / 1_000_000.0);
        }
        if !complete {
            if self.missing_input > 0 || self.missing_output > 0 {
                usage["incomplete"] = json!({
                    "input": self.missing_input,
                    "output": self.missing_output,
                });
            }
            if read_truncated {
                usage["read_truncated"] = json!(true);
            }
        }
        usage
    }
}

pub(super) async fn read(
    application: &CodingApplication,
    job: &JobId,
) -> Result<Value, CodingError> {
    let mut totals = Totals::default();
    let mut read_truncated = false;
    let mut after = None;
    for page in 0..MAX_PAGES {
        let attempts = application
            .store
            .list_coding_transport_attempts(job, after, PAGE)
            .await
            .map_err(CodingError::owner)?;
        for item in &attempts.items {
            if let Some(span) = item.span.as_ref() {
                totals.add(span.data());
            }
        }
        match attempts.next_cursor {
            Some(cursor) => after = Some(cursor),
            None => return Ok(totals.view(false)),
        }
        if page + 1 == MAX_PAGES {
            read_truncated = true;
        }
    }
    Ok(totals.view(read_truncated))
}
