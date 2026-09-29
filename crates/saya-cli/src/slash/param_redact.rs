//! Redaction of `--param` values from submitted command lines (D2, audit
//! A922-2): every persisted or displayed copy of a submitted line — the
//! input-history ring and file, the transcript's user block, the
//! queued-prompt preview — shows `--param name=…`, while execution receives
//! the original line with the real values. A pure function over the line;
//! callers decide which copies get the redacted form.

/// The `run` subcommand's flags: a token spelling one of these closes an
/// open `--param` value zone, exactly as the slash parser's `scan` does —
/// so what gets redacted is what the parser would have consumed as a
/// binding, and nothing the parser used as a value is ever displayed.
const RUN_FLAGS: [&str; 3] = ["--connection", "--revalidate", "--param"];

const PARAM_FLAG: &str = "--param";
const PARAM_PREFIX: &str = "--param=";

/// The displayed copy of a submitted line: `--param name=value` bindings
/// become `--param name=…` (a binding without `=` becomes `--param …`), on
/// any `/investigation`-family line. Everything else passes through
/// verbatim, whitespace included. Idempotent: the redacted line redacts to
/// itself.
pub(crate) fn redact_param_values(line: &str) -> String {
    if !is_investigation_command(line) {
        return line.to_owned();
    }
    let spans = token_spans(line);
    let mut out = String::with_capacity(line.len());
    let mut copied = 0;
    for &(start, end) in &spans {
        let token = &line[start..end];
        if token == PARAM_FLAG {
            let zone_end = zone_end(&spans, end, line);
            out.push_str(&line[copied..end]);
            let zone = &line[end..zone_end];
            let binding_start = end + (zone.len() - zone.trim_start().len());
            let binding_end = binding_start + zone.trim().len();
            if binding_start == binding_end {
                // A valueless `--param` displays as the malformed-entry shape.
                out.push_str(" …");
                copied = zone_end;
            } else {
                out.push_str(&line[end..binding_start]);
                out.push_str(&redact_binding(&line[binding_start..binding_end]));
                copied = binding_end;
            }
        } else if let Some(binding) = token.strip_prefix(PARAM_PREFIX) {
            out.push_str(&line[copied..start]);
            out.push_str(PARAM_PREFIX);
            out.push_str(&redact_binding(binding));
            copied = end;
        }
    }
    out.push_str(&line[copied..]);
    out
}

/// Whether the line parses as an investigation-family slash command: its
/// first token is `/investigation` or the `/investigations` alias. Both
/// spellings reach the same parser, so both get the same redaction.
fn is_investigation_command(line: &str) -> bool {
    matches!(
        line.split_whitespace().next(),
        Some("/investigation" | "/investigations")
    )
}

/// The byte where a `--param` value zone ends: the start of the first token
/// spelling one of [`RUN_FLAGS`] at or after `from`, or the line's end.
fn zone_end(spans: &[(usize, usize)], from: usize, line: &str) -> usize {
    for &(start, end) in spans {
        if start < from {
            continue;
        }
        if RUN_FLAGS.contains(&&line[start..end]) {
            return start;
        }
    }
    line.len()
}

/// One binding's displayed shape, the same the command Debug uses
/// (`cli_debug.rs`): `name=value` renders `name=…`, a binding without `=`
/// renders `…`.
fn redact_binding(binding: &str) -> String {
    match binding.split_once('=') {
        Some((name, _)) => format!("{name}=…"),
        None => "…".to_owned(),
    }
}

/// Whitespace-delimited token spans (start, end) of `line`, so the
/// reconstruction keeps the text between tokens verbatim. The same walk the
/// slash parser's flag grammar does; the parser stays the authority — this
/// only needs to agree with it on where a binding zone starts and ends.
fn token_spans(line: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < line.len() {
        let rest = &line[index..];
        index += rest.len() - rest.trim_start().len();
        if index >= line.len() {
            break;
        }
        let end = line[index..]
            .find(char::is_whitespace)
            .map_or(line.len(), |offset| index + offset);
        spans.push((index, end));
        index = end;
    }
    spans
}

#[cfg(test)]
#[path = "param_redact_tests.rs"]
mod tests;
