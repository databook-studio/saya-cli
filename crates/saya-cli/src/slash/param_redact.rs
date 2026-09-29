//! Redaction of `--param` values from submitted command lines (D2, audit
//! A922-2; D8, audit R061-1/R061-2): every persisted or displayed copy of a
//! submitted line — the input-history ring and file, the transcript's user
//! block, the queued-prompt preview — shows `--param name=…`, while execution
//! receives the original line. A pure function over the line; callers decide
//! which copies get the redacted form.
//!
//! The walk is total and idempotent (D8): each whitespace token is visited at
//! most once over non-overlapping spans — once a value zone is consumed, its
//! tokens are never revisited — so no `&str` input can panic it, and the
//! redacted line redacts to itself. Zone rules mirror the slash parser's
//! `scan` for the exact `--param` flag.

/// The `run` subcommand's flags: a token spelling one of these exactly closes
/// an open `--param` value zone, exactly as the slash parser's `scan` does —
/// so what gets redacted is what the parser would have consumed as a
/// binding, and nothing the parser used as a value is ever displayed.
const RUN_FLAGS: [&str; 3] = ["--connection", "--revalidate", "--param"];

const PARAM_FLAG: &str = "--param";
const PARAM_PREFIX: &str = "--param=";

/// The displayed copy of a submitted line: `--param name=value` bindings
/// become `--param name=…` (a binding without `=` becomes `--param …`), on
/// any `/investigation`-family line. The attached spelling `--param=…` —
/// which the parser rejects but history records before that rejection —
/// redacts its whole remainder through the next exact run flag, multiword
/// and quoted values included, shown as `--param=name=…` (`--param=…` if no
/// `=`). When in doubt a zone redacts more, never less. Everything else
/// passes through verbatim, whitespace included. Idempotent: the redacted
/// line redacts to itself.
pub(crate) fn redact_param_values(line: &str) -> String {
    if !is_investigation_command(line) {
        return line.to_owned();
    }
    let spans = token_spans(line);
    let mut out = String::with_capacity(line.len());
    let mut copied = 0;
    let mut index = 0;
    while index < spans.len() {
        let (start, end) = spans[index];
        let token = &line[start..end];
        if token == PARAM_FLAG {
            // The exact flag opens a value zone through the next exact run
            // flag or the line's end — the parser's scan rule. The whole
            // zone is one binding, consumed here; its tokens are never
            // revisited (R061-1 panicked on a revisit's backward slice).
            let zone_end = zone_end(&spans, end, line);
            out.push_str(&line[copied..end]);
            let zone = &line[end..zone_end];
            let binding_start = end + (zone.len() - zone.trim_start().len());
            let binding_end = binding_start + zone.trim().len();
            if binding_start == binding_end {
                // A valueless `--param` displays as the malformed-entry
                // shape; the zone's whitespace is left to the verbatim tail
                // so the next flag stays its own token (idempotence).
                out.push_str(" …");
                copied = end;
            } else {
                out.push_str(&line[end..binding_start]);
                out.push_str(&redact_binding(&line[binding_start..binding_end]));
                copied = binding_end;
            }
            index = spans.partition_point(|&(token_start, _)| token_start < zone_end);
        } else if let Some(binding) = token.strip_prefix(PARAM_PREFIX) {
            // The attached spelling: redact the whole remainder through the
            // next exact run flag — this token's binding names the display,
            // every later token in the region is consumed with it.
            let region_end = zone_end(&spans, end, line);
            out.push_str(&line[copied..start]);
            out.push_str(PARAM_PREFIX);
            out.push_str(&redact_binding(binding));
            copied = start + line[start..region_end].trim_end().len();
            index = spans.partition_point(|&(token_start, _)| token_start < region_end);
        } else {
            index += 1;
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

/// The byte where a value region ends: the start of the first token spelling
/// one of [`RUN_FLAGS`] at or after `from`, or the line's end. For an exact
/// `--param` this is its value zone's end (the parser's `scan` rule); for an
/// attached `--param=…` it is where the redacted remainder stops.
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
