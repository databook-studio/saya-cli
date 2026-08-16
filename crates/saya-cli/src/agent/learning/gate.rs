//! Heuristic proposal-worthiness gating for post-turn extraction — spec F Chunk 1.
//!
//! Evaluates whether a completed turn warrants invoking structured extraction:
//! 1. Override findings: confirmed claims contradicted by executed SQL.
//! 2. User assertion intent: explicit definitional or corrective language.
//! 3. Database object activity: tool observations touching database objects with non-trivial answers.

use crate::agent::learning::turn_record::TurnRecord;
use crate::agent::tools::DrainedObservations;

/// The decision produced by evaluating a turn's proposal worthiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum GatingDecision {
    /// The turn contains proposal-worthy facts; proceed with structured extraction.
    RunExtraction,
    /// Skip extraction to preserve tokens, cost, and latency.
    Skip { reason: &'static str },
}

impl GatingDecision {
    /// True when extraction should run.
    #[allow(dead_code)]
    pub fn is_run(&self) -> bool {
        matches!(self, Self::RunExtraction)
    }

    /// True when extraction should be skipped.
    #[allow(dead_code)]
    pub fn is_skip(&self) -> bool {
        matches!(self, Self::Skip { .. })
    }
}

/// Evaluates whether a completed turn warrants structured extraction.
#[allow(dead_code)]
pub struct ProposalGating;

impl ProposalGating {
    /// Evaluates proposal-worthiness against the turn record, observations, and override state.
    #[allow(dead_code)]
    pub fn evaluate(
        record: &TurnRecord,
        observations: &DrainedObservations,
        has_overrides: bool,
    ) -> GatingDecision {
        // 1. Override trigger: SQL contradicted a confirmed claim
        if has_overrides || !record.override_findings.is_empty() {
            return GatingDecision::RunExtraction;
        }

        // 2. User assertion trigger: prompt contains explicit definitional or corrective phrasing
        if contains_assertion_intent(&record.prompt) {
            return GatingDecision::RunExtraction;
        }

        // 3. Object activity trigger: database objects were inspected/queried with non-trivial answer
        let has_object_activity = !record.object_table.is_empty()
            || observations
                .observations
                .iter()
                .any(|o| !o.objects.is_empty());

        let non_trivial_answer = record.assistant_answer.trim().chars().count() >= 15;

        if has_object_activity && non_trivial_answer {
            return GatingDecision::RunExtraction;
        }

        if !has_object_activity {
            GatingDecision::Skip {
                reason: "no database objects touched or referenced",
            }
        } else {
            GatingDecision::Skip {
                reason: "trivial answer with no explicit assertions",
            }
        }
    }
}

/// Checks if prompt contains keywords or phrases indicative of semantic assertions.
#[allow(dead_code)]
pub(crate) fn contains_assertion_intent(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    const PATTERNS: &[&str] = &[
        "remember",
        "always ",
        "never ",
        "default ",
        "grain",
        "alias",
        "means ",
        "stands for",
        "is the ",
        "actually",
        "no,",
        "no, ",
        "note:",
        "primary key",
        "identifier",
        "timestamp",
        "dimension",
        "measure",
        "sensitive",
    ];
    PATTERNS.iter().any(|&p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::learning::turn_table::TurnObjectTable;
    use crate::agent::tools::{ObservationOutcome, ToolObservation};
    use saya_agent::OverrideFindingDto;
    use saya_types::ClaimId;

    fn empty_record(prompt: &str, answer: &str) -> TurnRecord {
        TurnRecord {
            prompt: prompt.into(),
            assistant_answer: answer.into(),
            object_table: TurnObjectTable::new(),
            user_corrections: Vec::new(),
            override_findings: Vec::new(),
            supplied_claims: Vec::new(),
        }
    }

    fn record_with_object(prompt: &str, answer: &str) -> TurnRecord {
        let mut table = TurnObjectTable::new();
        table.register("primary", "catalog.public.orders", &[]);
        TurnRecord {
            prompt: prompt.into(),
            assistant_answer: answer.into(),
            object_table: table,
            user_corrections: Vec::new(),
            override_findings: Vec::new(),
            supplied_claims: Vec::new(),
        }
    }

    fn empty_obs() -> DrainedObservations {
        DrainedObservations {
            observations: Vec::new(),
            truncated: false,
        }
    }

    #[test]
    fn test_gating_triggers_on_sql_observations() {
        let record = record_with_object(
            "select orders",
            "Here are the top 5 customer orders from public.orders.",
        );
        let obs = DrainedObservations {
            observations: vec![ToolObservation {
                tool: "bounded_sql_query".into(),
                outcome: ObservationOutcome::Succeeded,
                profile: None,
                objects: vec![vec!["catalog".into(), "public".into(), "orders".into()]],
                columns: vec!["id".into()],
                row_count: Some(5),
                truncated: Some(false),
                references_partial: false,
            }],
            truncated: false,
        };

        let decision = ProposalGating::evaluate(&record, &obs, false);
        assert_eq!(decision, GatingDecision::RunExtraction);
        assert!(decision.is_run());
    }

    #[test]
    fn test_gating_triggers_on_user_assertion_keywords() {
        let record = empty_record(
            "remember that orders.shipped_at is default time",
            "Understood, I will keep that in mind.",
        );
        let decision = ProposalGating::evaluate(&record, &empty_obs(), false);
        assert_eq!(decision, GatingDecision::RunExtraction);
    }

    #[test]
    fn test_gating_triggers_on_override_findings() {
        let mut record = empty_record("query orders", "Done.");
        record.override_findings.push(OverrideFindingDto {
            claim_id: ClaimId::parse("c-1").unwrap(),
            kind: "default_time_column".into(),
            claimed_value: "created_at".into(),
            observed_columns: vec!["shipped_at".into()],
        });

        let decision = ProposalGating::evaluate(&record, &empty_obs(), false);
        assert_eq!(decision, GatingDecision::RunExtraction);
    }

    #[test]
    fn test_gating_skips_trivial_conversations() {
        let record1 = empty_record("hello", "Hello! How can I help you?");
        assert_eq!(
            ProposalGating::evaluate(&record1, &empty_obs(), false),
            GatingDecision::Skip {
                reason: "no database objects touched or referenced"
            }
        );

        let record2 = empty_record("what is 2+2", "2 + 2 = 4");
        assert!(ProposalGating::evaluate(&record2, &empty_obs(), false).is_skip());

        let record_empty_answer = record_with_object("select orders", "");
        assert_eq!(
            ProposalGating::evaluate(&record_empty_answer, &empty_obs(), false),
            GatingDecision::Skip {
                reason: "trivial answer with no explicit assertions"
            }
        );
    }
}
