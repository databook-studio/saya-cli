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

/// The one consent grammar: `/mode` and its `/approvals` alias both read it,
/// so the six values are stated once. Case-sensitive; an unknown word names
/// every value that parses rather than reading as a silent default.
fn parse_mode(value: &str) -> Result<Option<PermissionMode>, SlashParseError> {
    if value.is_empty() {
        return Ok(None);
    }
    crate::interactive::session::permissions::mode::parse(value)
        .map(Some)
        .ok_or_else(|| {
            SlashParseError(format!(
                "invalid permission mode: {value} (expected ask, accept-edits, plan, bypass, \
                 read-only, or never)"
            ))
        })
}
