fn parse_bool(value: &str) -> Result<Option<bool>, SlashParseError> {
    if value.is_empty() {
        return Ok(None);
    }
    match value {
        "on" | "true" | "enable" => Ok(Some(true)),
        "off" | "false" | "disable" => Ok(Some(false)),
        _ => Err(SlashParseError("expected on or off".into())),
    }
}

fn parse_approval(value: &str) -> Result<Option<ApprovalPolicy>, SlashParseError> {
    if value.is_empty() {
        return Ok(None);
    }
    ApprovalPolicy::from_str(value)
        .map(Some)
        .map_err(|error| SlashParseError(error.to_string()))
}

fn parse_mode(value: &str) -> Result<Option<AgentMode>, SlashParseError> {
    if value.is_empty() {
        return Ok(None);
    }
    AgentMode::from_str(value).map(Some).map_err(|_| {
        SlashParseError(format!(
            "invalid agent mode: {value} (expected plan or build)"
        ))
    })
}
