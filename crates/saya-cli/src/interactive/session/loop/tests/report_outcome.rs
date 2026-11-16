#[cfg(test)]
mod report_outcome_tests {
    use super::*;

    /// `/report` in a headless turn is refused as TUI-only and the turn ends
    /// errored — exactly like `/export`.
    #[test]
    fn report_turn_outcome_is_errored_like_export() {
        let request = crate::slash::ReportRequest {
            path: "out.md".into(),
            rows: None,
            overwrite: false,
        };
        assert!(
            matches!(
                action_outcome(&SessionAction::Report(request)),
                TurnOutcome::Errored
            ),
            "a headless /report turn ends errored"
        );
    }
}
