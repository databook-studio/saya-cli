pub(crate) fn redact(value: &str) -> String {
    redact_urls(&redact_markers(value))
}

fn redact_markers(value: &str) -> String {
    let markers = ["password=", "api_key=", "token=", "secret="];
    let lower = value.to_ascii_lowercase();
    let mut output = String::new();
    let mut cursor = 0;
    while cursor < value.len() {
        let Some((start, marker)) = markers
            .iter()
            .filter_map(|marker| {
                lower[cursor..]
                    .find(marker)
                    .map(|offset| (cursor + offset, *marker))
            })
            .min_by_key(|(start, _)| *start)
        else {
            break;
        };
        output.push_str(&value[cursor..start + marker.len()]);
        let value_start = start + marker.len();
        let end = value[value_start..]
            .find(|character: char| {
                character.is_whitespace() || character == '&' || character == ';'
            })
            .map(|offset| value_start + offset)
            .unwrap_or(value.len());
        output.push_str("[redacted]");
        cursor = end;
    }
    output.push_str(&value[cursor..]);
    output
}

fn redact_urls(value: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    while let Some(offset) = value[cursor..].find("://") {
        let scheme = cursor + offset;
        let auth_start = scheme + 3;
        let rest = &value[auth_start..];
        let Some(at_offset) = rest.find('@') else {
            break;
        };
        let boundary = rest
            .find(|character: char| "/?# \t\r\n".contains(character))
            .unwrap_or(rest.len());
        if at_offset >= boundary {
            cursor = auth_start;
            continue;
        }
        let at = auth_start + at_offset;
        output.push_str(&value[cursor..auth_start]);
        output.push_str("[redacted]@");
        cursor = at + 1;
    }
    output.push_str(&value[cursor..]);
    output
}
