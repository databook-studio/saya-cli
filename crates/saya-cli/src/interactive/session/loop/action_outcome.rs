fn action_outcome(action: &SessionAction) -> TurnOutcome {
    match action {
        SessionAction::Exit => TurnOutcome::Exit,
        SessionAction::Error(_)
        | SessionAction::Cancelled
        | SessionAction::NotImplemented(_)
        | SessionAction::Export(_)
        | SessionAction::Report(_)
        | SessionAction::Chart(_)
        | SessionAction::Explain(_) => TurnOutcome::Errored,
        _ => TurnOutcome::Completed,
    }
}
