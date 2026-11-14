use saya_types::redact;

/// Cap on the failure reason carried into the human-facing summary, in
/// characters. Long enough to name a safety rejection (the canonical refusal
/// is well under half this) and short enough that one failure cannot flood
/// the transcript or a persisted session. The bound is the summary only —
/// the model's JSON still carries the full error.
pub(super) const MAX_FAILURE_REASON_CHARS: usize = 200;

/// Appends a failure reason to the generic failure summary so a safety-layer
/// refusal reads differently from a runtime failure. The reason is the
/// untrusted error text, so it is redacted (secret-shaped material must not
/// reach `TerminalEvent` payloads, per `security.md`), single-lined, and
/// bounded at [`MAX_FAILURE_REASON_CHARS`] with the existing `…` marker.
/// Keeps "failed" in model-facing wording and offers no override — the line
/// explains what was refused, never how to force it. Typed outcomes, rather
/// than this substring, drive turn-tool status and failure memory.
pub(super) fn failure_summary_with_reason(failed: &str, error: &str) -> String {
    let reason = bound_failure_reason(&redact(error));
    if reason.is_empty() {
        return failed.to_owned();
    }
    format!("{failed} — {reason}")
}

/// Single-lines `reason` and cuts it to [`MAX_FAILURE_REASON_CHARS`]
/// characters, marking the cut with `…`. Operates on characters (not bytes)
/// so the slice never splits a multi-byte sequence, and on an already
/// redacted string — redaction runs first so secret-shaped material cannot
/// hide inside the truncated tail.
fn bound_failure_reason(reason: &str) -> String {
    let single_line = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    let single_line = single_line.trim();
    if single_line.is_empty() {
        return String::new();
    }
    let bounded: String = single_line.chars().take(MAX_FAILURE_REASON_CHARS).collect();
    if bounded.len() < single_line.len() {
        format!("{bounded}…")
    } else {
        bounded
    }
}
