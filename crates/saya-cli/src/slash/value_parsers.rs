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

fn parse_mode(value: &str) -> Result<Option<PermissionMode>, SlashParseError> {
    if value.is_empty() {
        return Ok(None);
    }
    crate::interactive::session::permissions::mode::parse(value)
        .map(Some)
        .ok_or_else(|| {
            SlashParseError(format!(
                "invalid permission mode: {value} (expected ask, accept-edits, plan, or bypass)"
            ))
        })
}
