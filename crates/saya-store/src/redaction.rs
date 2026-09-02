pub fn redact(value: &str) -> String {
    redact_urls(&redact_headers_and_keys(&redact_markers(value)))
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

/// Credential headers and private-key blocks are redacted wholesale: unlike
/// `key=value` markers, a header value runs to end of line (e.g.
/// `Authorization: Bearer <token>`), and a PEM block spans lines. Backported
/// from the knowledge-admission gate so transcripts get the same coverage.
///
/// Three holes closed here vs. the `dd3d110` version:
/// - a header is matched anywhere in a line, not only at the
///   start — pasted `curl -H '...'` is the real shape;
/// - a header value is redacted to end of line, and the
///   `[redacted]` marker keeps its closing bracket when the line continues;
/// - a private-key block with a `BEGIN` and no `END` — the truncated case that
///   byte-capped transcripts make the *expected* one — is redacted to the end
///   of the buffer rather than emitted verbatim.
fn redact_headers_and_keys(value: &str) -> String {
    const HEADERS: [&str; 5] = [
        "authorization:",
        "proxy-authorization:",
        "x-api-key:",
        "x-auth-token:",
        "cookie:",
    ];

    // First pass: credential headers, line by line. A header is matched
    // wherever the `name:` shape appears in the line, but the
    // colon is what distinguishes it from the bare word in a SQL comment
    //. Only the earliest match matters: redacting its value to
    // end of line consumes the rest of the line, so any later header name in
    // the same line is swallowed with it.
    let mut output = String::new();
    for line in value.split_inclusive('\n') {
        if let Some(name_end) = find_earliest_header(line, &HEADERS) {
            // `name_end` is the byte just past the colon; redact the rest.
            let (kept, rest) = line.split_at(name_end);
            output.push_str(kept);
            output.push_str(" [redacted]");
            // Preserve the trailing newline, if any, without touching the
            // closing bracket we just wrote. (A CRLF terminator loses its
            // `\r` here — matching the prior behaviour; transcripts are
            // `\n`-delimited and this case is not in the spec.)
            if rest.ends_with('\n') {
                output.push('\n');
            }
        } else {
            output.push_str(line);
        }
    }

    // Second pass: PEM private-key blocks. A block is the region from a
    // `-----BEGIN` line through the following `-----END` line (end-of-line on
    // the END marker). If no END marker follows the BEGIN, the block runs to
    // the end of the buffer — a redactor fails closed.
    let mut final_output = String::new();
    let mut cursor = 0;
    while let Some(begin) = find_ignore_ascii_case(&output[cursor..], "-----begin") {
        let begin = cursor + begin;
        let block_end = match find_ignore_ascii_case(&output[begin..], "-----end") {
            Some(end_offset) => {
                let end_marker = begin + end_offset;
                output[end_marker..]
                    .find('\n')
                    .map(|offset| end_marker + offset)
                    .unwrap_or(output.len())
            }
            None => output.len(),
        };
        let block = &output[begin..block_end];
        final_output.push_str(&output[cursor..begin]);
        if find_ignore_ascii_case(block, "private key").is_some() {
            final_output.push_str("[redacted private key]");
            if block.ends_with('\n') {
                final_output.push('\n');
            }
        } else {
            final_output.push_str(block);
        }
        cursor = block_end;
        if cursor >= output.len() {
            break;
        }
    }
    final_output.push_str(&output[cursor.min(output.len())..]);
    final_output
}

/// Returns the byte index in `line` just past the first `name:` header match
/// (case-insensitive), or `None`. The match must be the full header name
/// immediately followed by a colon, so the bare word `Authorization` without a
/// colon never matches.
fn find_earliest_header(line: &str, headers: &[&str]) -> Option<usize> {
    let bytes = line.as_bytes();
    headers
        .iter()
        .filter_map(|header| {
            let needle = header.as_bytes();
            let mut start = 0;
            while start + needle.len() <= bytes.len() {
                if eq_ignore_ascii_case_slice(&bytes[start..start + needle.len()], needle) {
                    return Some(start + needle.len());
                }
                start += 1;
            }
            None
        })
        .min()
}

/// Case-insensitive substring search for an ASCII needle. Avoids allocating a
/// lowercase copy of the haystack (the previous code lowercased the whole
/// buffer and each block).
fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || n.len() > h.len() {
        return None;
    }
    let last = h.len() - n.len();
    let mut i = 0;
    while i <= last {
        if eq_ignore_ascii_case_slice(&h[i..i + n.len()], n) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn eq_ignore_ascii_case_slice(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn credential_headers_redact_their_values_to_eol() {
        assert_eq!(
            redact("Authorization: Bearer sk-live-abc123"),
            "Authorization: [redacted]"
        );
        assert_eq!(redact("X-API-Key: hunter2 extra"), "X-API-Key: [redacted]");
        assert_eq!(redact("Cookie: session=xyz; path=/"), "Cookie: [redacted]");
        // Ordinary lines pass through untouched.
        assert_eq!(
            redact("SELECT 1 -- Authorization"),
            "SELECT 1 -- Authorization"
        );
    }

    #[test]
    fn credential_header_inside_a_quoted_shell_argument_is_redacted() {
        // Row 1 of the spec table: a pasted `curl -H '...'` carries the header
        // mid-line, not at the start.
        let out = redact("curl -H 'Authorization: Bearer sk-live-SECRET' https://x");
        assert!(
            !out.contains("sk-live-SECRET"),
            "live token survived redaction: {out:?}"
        );
        assert!(
            out.contains("[redacted]"),
            "header value was not redacted: {out:?}"
        );
    }

    #[test]
    fn truncated_private_key_block_is_redacted_to_end_of_buffer() {
        // Row 2 of the spec table: a BEGIN with no matching END must be redacted
        // from the BEGIN marker to the end of the buffer, not emitted verbatim.
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowSECRET\nmore";
        let out = redact(pem);
        assert!(
            !out.contains("MIIEowSECRET"),
            "truncated key body survived: {out:?}"
        );
        assert!(
            !out.contains("more"),
            "truncated key tail survived: {out:?}"
        );
        assert!(
            out.contains("[redacted private key]"),
            "no redaction marker emitted: {out:?}"
        );
    }

    #[test]
    fn redacted_header_keeps_its_closing_bracket_across_newline() {
        // Row 3 of the spec table: the `[redacted]` must stay well-formed when
        // the header line is followed by more input.
        let out = redact("Authorization: Bearer x\nnext line");
        assert_eq!(out, "Authorization: [redacted]\nnext line");
    }

    #[test]
    fn truncated_private_key_emits_nothing_after_begin_marker() {
        // Specifically: a PEM block with a PRIVATE KEY BEGIN and no
        // closing marker leaks nothing after the BEGIN marker.
        let pem = "before\n-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIEowSECRET\ntail-without-end";
        let out = redact(pem);
        assert!(out.contains("before"), "non-secret prefix lost: {out:?}");
        assert!(
            !out.contains("MIIEowSECRET"),
            "key body leaked after BEGIN marker: {out:?}"
        );
        assert!(
            !out.contains("tail-without-end"),
            "key tail leaked after BEGIN marker: {out:?}"
        );
        assert!(out.contains("[redacted private key]"));
    }

    #[test]
    fn certificate_blocks_and_authorization_prose_stay_intact() {
        // Non-secret content is never destroyed.
        let cert = "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----";
        assert_eq!(redact(cert), cert);
        assert_eq!(
            redact("SELECT 1 -- Authorization"),
            "SELECT 1 -- Authorization"
        );
    }

    #[test]
    fn pem_private_key_blocks_are_removed_wholesale() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAK\nabcdef==\n-----END RSA PRIVATE KEY-----\nafter";
        let out = redact(pem);
        assert!(!out.contains("MIIEow"));
        assert!(out.contains("[redacted private key]"));
        assert!(out.contains("after"));
        // Public certs are left alone (not secret material).
        let cert = "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----";
        assert_eq!(redact(cert), cert);
    }
}
