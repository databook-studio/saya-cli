#[cfg(test)]
mod report_outcome_tests {
    use super::*;

    /// `/report` is `/export --format md`'s alias, so a headless turn spelled
    /// that way is refused as TUI-only and ends errored — exactly like
    /// `/export`.
    #[test]
    fn report_turn_outcome_is_errored_like_export() {
        let request = crate::slash::ExportRequest {
            format: Some(crate::slash::ExportFormat::Markdown),
            path: "out.md".into(),
            via_report_alias: true,
            ..crate::slash::ExportRequest::default()
        };
        assert!(
            matches!(
                action_outcome(&SessionAction::Export(request)),
                TurnOutcome::Errored
            ),
            "a headless /report turn ends errored"
        );
    }
}
