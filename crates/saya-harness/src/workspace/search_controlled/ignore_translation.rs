//! Compatibility over typed gix patterns and globset's uncompiled regex output.
//! This is not an ignore-line parser or a native Wildmatch execution path.
use globset::GlobBuilder;

const MARKER: &str = "__SAYA_ASCII_CLASS_";
const CLASSES: &[&str] = &[
    "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space",
    "upper", "xdigit",
];

pub(super) fn translate(text: &str, basename: bool) -> Result<String, ()> {
    if text.contains(MARKER) {
        return Err(());
    }
    let mut converted = String::new();
    let mut substitutions = Vec::new();
    let mut input = text.chars().peekable();
    let mut class = false;
    let mut first_in_class = false;
    while let Some(character) = input.next() {
        if character == '\\' {
            if class {
                return Err(());
            } // globset does not implement Git class escapes.
            converted.push(character);
            converted.push(input.next().ok_or(())?);
            continue;
        }
        if character == '[' && !class {
            let remaining: String = input.clone().collect();
            if let Some((length, name, negative)) = posix_class(&remaining)? {
                let marker = format!("{MARKER}{}__", substitutions.len());
                converted.push_str(&marker);
                substitutions.push((
                    marker,
                    format!("[{}[:{name}:]]", if negative { "^" } else { "" }),
                ));
                for _ in 0..length {
                    input.next();
                }
                continue;
            }
            class = true;
            first_in_class = true;
            converted.push(character);
            continue;
        }
        if class {
            if character == ']' && !first_in_class {
                class = false;
            }
            if !matches!(character, '!' | '^') || !first_in_class {
                first_in_class = false;
            }
            converted.push(character);
        } else {
            if matches!(character, '{' | '}') {
                converted.push('\\');
            }
            converted.push(character);
        }
    }
    if class {
        return Err(());
    }
    let normalized = if basename {
        format!("**/{converted}")
    } else {
        converted
    };
    let glob = GlobBuilder::new(&normalized)
        .literal_separator(true)
        .backslash_escape(true)
        .case_insensitive(false)
        .build()
        .map_err(|_| ())?;
    let mut regex = glob.regex().to_owned();
    for (marker, replacement) in substitutions {
        regex = regex.replace(&marker, &replacement);
    }
    // Git's pathname matching never allows a character class to consume '/'.
    // Operate only on the public library-generated regex, not raw ignore lines.
    let regex = format!("(?s){}", exclude_separator(&regex)?);
    if regex.len() > 8192 {
        return Err(());
    }
    Ok(regex)
}

fn posix_class(rest: &str) -> Result<Option<(usize, &str, bool)>, ()> {
    let (body, negative, prefix) = if let Some(body) = rest.strip_prefix("![:") {
        (body, true, 3)
    } else if let Some(body) = rest.strip_prefix("^[:") {
        (body, true, 3)
    } else if let Some(body) = rest.strip_prefix("[:") {
        (body, false, 2)
    } else {
        // Mixed POSIX classes have a different grammar; never silently expand
        // them using globset's incompatible bracket interpretation.
        let end = rest.find(']').unwrap_or(rest.len());
        return if rest[..end].contains("[:") {
            Err(())
        } else {
            Ok(None)
        };
    };
    let end = body.find(":]]").ok_or(())?;
    let name = &body[..end];
    if !CLASSES.contains(&name) {
        return Err(());
    }
    Ok(Some((prefix + end + 3, name, negative)))
}

fn exclude_separator(regex: &str) -> Result<String, ()> {
    let chars: Vec<char> = regex.chars().collect();
    let mut result = String::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] == '\\' {
            result.push(chars[at]);
            at += 1;
            result.push(*chars.get(at).ok_or(())?);
            at += 1;
        } else if chars[at] == '[' {
            let start = at;
            at += 1;
            let mut nested = 0;
            while at < chars.len() {
                if chars[at] == '\\' {
                    at += 2;
                    continue;
                }
                if chars[at] == '[' {
                    nested += 1;
                }
                if chars[at] == ']' {
                    if nested == 0 {
                        break;
                    }
                    nested -= 1;
                }
                at += 1;
            }
            if at >= chars.len() {
                return Err(());
            }
            result.push('[');
            result.extend(&chars[start..=at]);
            result.push_str("&&[^/]]");
            at += 1;
        } else {
            result.push(chars[at]);
            at += 1;
        }
        if result.len() > 8192 {
            return Err(());
        }
    }
    Ok(result)
}
