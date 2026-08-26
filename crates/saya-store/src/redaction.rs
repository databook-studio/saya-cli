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
fn redact_headers_and_keys(value: &str) -> String {
    const HEADERS: [&str; 5] = [
        "authorization:",
        "proxy-authorization:",
        "x-api-key:",
        "x-auth-token:",
        "cookie:",
    ];
    let mut output = String::new();
    for line in value.split_inclusive('\n') {
        let lower = line.to_ascii_lowercase();
        if HEADERS
            .iter()
            .any(|header| lower.trim_start().starts_with(*header))
        {
            let trimmed = line.trim_start();
            let Some(colon) = trimmed.find(':') else {
                output.push_str(line);
                continue;
            };
            let indent_len = line.len() - trimmed.len();
            output.push_str(&line[..indent_len]);
            output.push_str(&trimmed[..colon + 1]);
            output.push_str(" [redacted]");
            if trimmed.ends_with('\n') {
                output.pop();
                output.push('\n');
            }
        } else {
            output.push_str(line);
        }
    }
    // PEM private-key blocks: replace the whole delimited region.
    let lower = output.to_ascii_lowercase();
    let mut final_output = String::new();
    let mut cursor = 0;
    while let Some(begin) = lower[cursor..].find("-----begin") {
        let begin = cursor + begin;
        let Some(end_marker_offset) = lower[begin..].find("-----end") else {
            break;
        };
        let end_line_end = lower[begin + end_marker_offset..]
            .find('\n')
            .map(|offset| begin + end_marker_offset + offset)
            .unwrap_or(output.len());
        let block = &output[begin..end_line_end];
        let block_lower = block.to_ascii_lowercase();
        final_output.push_str(&output[cursor..begin]);
        if block_lower.contains("private key") {
            let suffix = if block.ends_with('\n') { "\n" } else { "" };
            final_output.push_str("[redacted private key]");
            final_output.push_str(suffix);
        } else {
            final_output.push_str(block);
        }
        cursor = end_line_end;
        if cursor >= output.len() {
            break;
        }
    }
    final_output.push_str(&output[cursor.min(output.len())..]);
    final_output
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
