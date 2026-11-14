// Reading one command file's text: the name it may claim, and the optional
// `---` front matter around its prompt body. Nothing here interprets a
// setting — `description` is a label, and every other key is ignored.

/// A command name: lowercase letters, digits, and `-`, at most
/// `MAX_COMMAND_NAME_CHARS` of them, starting with a letter or digit. Anything
/// else is not a name a surface could type.
fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_COMMAND_NAME_CHARS
        && name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Splits the optional `---` front matter from the prompt body. `Ok` carries
/// the key/value lines (empty when there is no front matter) and the body;
/// `Err` is the refusal reason for front matter that opens but never closes,
/// or a line that is not `key: value`.
fn split_front_matter(text: &str) -> Result<(Vec<(String, String)>, String), String> {
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().unwrap_or_default();
    if first.trim_end() != "---" {
        return Ok((Vec::new(), text.to_string()));
    }
    let mut offset = first.len();
    let mut keys = Vec::new();
    for line in lines {
        offset += line.len();
        let trimmed = line.trim_end();
        if trimmed == "---" {
            return Ok((keys, text[offset..].to_string()));
        }
        let trimmed = trimmed.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            return Err(format!("front matter line `{trimmed}` is not `key: value`"));
        };
        keys.push((key.trim().to_string(), unquote(value.trim())));
    }
    Err("its front matter opens with `---` but never closes".into())
}

/// Strips one pair of surrounding quotes, so `description: "Review"` reads as
/// `Review`. Nothing else is interpreted.
fn unquote(value: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.to_string();
        }
    }
    value.to_string()
}
