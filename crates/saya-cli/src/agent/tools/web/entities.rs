//! Character references: the named entities a fetched document actually uses,
//! and the numeric forms beside them.
//!
//! The table is deliberately small and closed. An entity it does not know — or
//! one a bounded read cut in half — is left exactly as the page wrote it,
//! because the converter never guesses at what a page meant.

/// The named entities a fetched document actually uses, plus the numeric forms
/// below. `&shy;` decodes to nothing, which is what a soft hyphen renders as.
fn named(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "shy" => "",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "middot" => "·",
        "bull" => "•",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "laquo" => "«",
        "raquo" => "»",
        "times" => "×",
        "divide" => "÷",
        "plusmn" => "±",
        "minus" => "−",
        "deg" => "°",
        "micro" => "µ",
        "frac12" => "½",
        "frac14" => "¼",
        "sup2" => "²",
        "sup3" => "³",
        "euro" => "€",
        "pound" => "£",
        "yen" => "¥",
        "cent" => "¢",
        "sect" => "§",
        "para" => "¶",
        "dagger" => "†",
        "prime" => "′",
        "permil" => "‰",
        _ => return None,
    })
}

/// A numeric character reference: `&#8212;` or `&#x2014;`.
fn numeric(name: &str) -> Option<String> {
    let digits = name.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse().ok()?,
    };
    char::from_u32(code).map(String::from)
}

/// Decodes the entities in one run of text. An entity the tables do not know —
/// or one the bounded read cut in half — is left exactly as the page wrote it,
/// because the converter never guesses at what a page meant.
pub(super) fn decode(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_owned();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail
            .char_indices()
            .take(32)
            .find(|(_, character)| *character == ';')
            .map(|(at, _)| at);
        match end.and_then(|end| {
            named(&tail[1..end])
                .map(str::to_owned)
                .or_else(|| numeric(&tail[1..end]))
        }) {
            Some(decoded) => {
                out.push_str(&decoded);
                rest = &tail[end.expect("the entity's terminator was found") + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}
