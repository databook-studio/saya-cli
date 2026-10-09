/// Reads a required string argument.
pub(crate) fn required_string<'a>(
    request: &'a CallToolRequestParams,
    key: &str,
) -> Result<&'a str, ErrorData> {
    string_arg(request, key)?
        .ok_or_else(|| ErrorData::invalid_params(format!("{key} is required"), None))
}

/// Reads an optional string argument; a non-string value is a parameter error.
pub(crate) fn optional_string<'a>(
    request: &'a CallToolRequestParams,
    key: &str,
) -> Result<Option<&'a str>, ErrorData> {
    string_arg(request, key)
}

/// Reads an optional `{ name: string }` map as the canonical `name=value`
/// strings the run command binds (deliverable Dc-1: `investigation_run`
/// params). Absent or empty binds nothing; a non-object map, or a non-string
/// value in it, is a parameter error — the values themselves are only ever
/// handed to the same typed parser the CLI uses and are never echoed.
pub(crate) fn optional_string_map(
    request: &CallToolRequestParams,
    key: &str,
) -> Result<Vec<String>, ErrorData> {
    let Some(arguments) = request.arguments.as_ref() else {
        return Ok(Vec::new());
    };
    let Some(value) = arguments.get(key) else {
        return Ok(Vec::new());
    };
    let Some(map) = value.as_object() else {
        return Err(ErrorData::invalid_params(
            format!("{key} must be an object of string values"),
            None,
        ));
    };
    map.iter()
        .map(|(name, value)| {
            value.as_str().map_or_else(
                || {
                    Err(ErrorData::invalid_params(
                        format!("{key}.{name} must be a string"),
                        None,
                    ))
                },
                |text| Ok(format!("{name}={text}")),
            )
        })
        .collect()
}

fn string_arg<'a>(
    request: &'a CallToolRequestParams,
    key: &str,
) -> Result<Option<&'a str>, ErrorData> {
    let Some(arguments) = request.arguments.as_ref() else {
        return Ok(None);
    };
    match arguments.get(key) {
        None => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(ErrorData::invalid_params(
            format!("{key} must be a string"),
            None,
        )),
    }
}
