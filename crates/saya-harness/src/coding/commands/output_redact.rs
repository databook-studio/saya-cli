//! The streaming redactor the per-stream writer threads run, in two
//! stages: stage A scrubs known secrets from a RAW holdback of
//! `max_secret - 1` bytes — a scrub span crossing the cut moves the cut
//! to its start, so no value is split — and stage B applies the pattern
//! pass to carried redacted text with holdback `MIN_KEEP + KEEP_SLACK`.
//! Secrets are never re-scanned after stage A, so marker text in carried
//! output cannot hide or recreate a value.
//!
//! Byte-exact carry: a window is never lossily decoded. The raw bytes of a
//! push are joined to a small raw tail — an incomplete UTF-8 sequence, at
//! most 3 bytes, held from the previous push — and only the complete-UTF-8
//! prefix of the join is decoded (`Utf8Error::valid_up_to`); the
//! incomplete trailing sequence (`error_len() == None`) stays raw in
//! `tail_raw` and is prepended to the next push, while a genuinely invalid
//! sequence (`error_len() == Some`) becomes one U+FFFD at that point only,
//! so invalid bytes never block progress. The decoded text joins the raw
//! holdback for stage A; stage B runs over carried redacted text, which
//! never needs re-decoding.
//!
//! Residual, documented by design. `saya_types::redact` is a best-effort
//! pattern pass: like the output ring, which cannot match a value split
//! across its cap boundary (runner/output.rs), this window cannot match a
//! single PATTERN match longer than the holdback. That residual is accepted
//! for pattern matches only. It is NOT accepted for KNOWN secret values —
//! the injected credential registry: every value up to [`MAX_SECRET_BYTES`]
//! is airtight, because stage A scrubs over the RAW held text — the last
//! `max_secret - 1` bytes joined to the new decoded text — so a value that
//! began in an earlier push still matches whole (a span crossing the
//! stage-A cut moves the cut to its start, so no value is split). An
//! invalid byte sequence breaks a value's contiguity in the decoded text,
//! exactly like a stream that never wrote the value contiguously.
//!
//! Marker collisions: carried text can contain two redaction literals —
//! [`REDACTION_MARKER_TEXT`], which `scrub_secrets` (runner/output.rs)
//! writes for every known secret occurrence, and [`PRIVATE_KEY_MARKER_TEXT`],
//! which the pattern pass (`saya_types::redact`) writes for private-key
//! blocks — so a value that sits inside either marker, contains either
//! marker, or overlaps a marker boundary would re-appear inside carried
//! text — possibly SYNTHESIZED by the redaction itself, e.g. "token=abc X"
//! becoming "token=[redacted] X" — and break the no-fragment proof:
//! [`Redactor::new`] refuses such a set and the writer seals the store
//! with zero stored bytes. Residual: text the pattern pass emits outside
//! its markers (a header name, say) could in theory coincide with a
//! registered value; accepted and documented, not prevented.

use crate::runner::output::{redact_chunk, scrub_secrets, secret_spans};

/// The longest secret value accepted for streaming redaction. A longer
/// value cannot be protected by any bounded holdback window, so the
/// redactor flags itself [`Redactor::is_refused`] and the writer seals the
/// store with zero stored bytes instead of emitting unredacted bytes.
const MAX_SECRET_BYTES: usize = 4096;

/// The shortest secret value accepted. A shorter value (including an empty
/// one) cannot be redacted soundly in a streamed store: it cannot be kept
/// out of an emission cut and may sit inside carried text.
const MIN_SECRET_BYTES: usize = 4;

/// The bound on RAW bytes stage A may hold after a push. A legitimate hold
/// is at most `max_secret - 1` plus one pump chunk, so a longer hold means
/// an unbounded overlapping match chain (e.g. secret "aaaa" over a stream
/// of 'a'); the redactor then seals itself.
const MAX_STAGE_A_HOLD: usize = 64 * 1024;

/// The holdback floor, in bytes of redacted text: even when every known
/// value is short the window is this wide, so ordinary pattern matches
/// survive the cut (longer ones are the documented residual).
const MIN_KEEP: usize = 1024;

/// Spare holdback above the [`MIN_KEEP`] floor: the stage-B holdback is
/// `MIN_KEEP + KEEP_SLACK`, so a pattern match a little longer than the
/// floor can still survive the cut despite char-boundary backoff. Known
/// secret values are protected by stage A, not by this slack.
const KEEP_SLACK: usize = 63;

/// Must equal the marker in runner/output.rs scrub_secrets: the literal
/// every known secret occurrence is replaced with, and therefore the
/// literal carried text may contain and must never be damaged by a secret
/// match. Guarded by the marker-equality unit test.
const REDACTION_MARKER_TEXT: &str = "[redacted]";

/// Must equal the other marker the pattern pass (`saya_types::redact`)
/// writes — the private-key literal — so a value the pattern pass could
/// wrap in it, or one carrying it verbatim, is refused like a
/// [`REDACTION_MARKER_TEXT`] collision. Guarded by the marker-equality
/// unit test.
const PRIVATE_KEY_MARKER_TEXT: &str = "[redacted private key]";

/// Redaction state for one stream's writer: the carried redacted text held
/// back past the emission cut, the RAW holdback of decoded text not yet
/// scrubbed for secrets, the raw bytes of an incomplete UTF-8 trailing
/// sequence, the stage-B holdback size, the longest known secret length,
/// and the secret values the run's environment was built from.
pub(super) struct Redactor {
    pending: String,
    held_raw: String,
    tail_raw: Vec<u8>,
    keep: usize,
    max_secret: usize,
    secrets: Vec<String>,
    refused: bool,
}

impl Redactor {
    /// Builds the two-stage redactor. Stage A scrubs known secrets from a
    /// RAW holdback of `max_secret - 1` bytes, where a scrub span crossing
    /// the cut moves the cut to its start, so no value is split; stage B
    /// applies the pattern pass to carried redacted text with holdback
    /// `MIN_KEEP + KEEP_SLACK`. A secret longer than [`MAX_SECRET_BYTES`],
    /// shorter than [`MIN_SECRET_BYTES`], or one that collides with either
    /// redaction marker — [`REDACTION_MARKER_TEXT`] from the value pass or
    /// [`PRIVATE_KEY_MARKER_TEXT`] from the pattern pass (sits inside one,
    /// contains one, or overlaps a marker boundary, see
    /// [`spans_marker_boundary`]) — cannot be redacted soundly in a
    /// streamed store: the redactor refuses such a set and seals
    /// everything instead.
    pub(super) fn new(secrets: &[String]) -> Self {
        let collides = secrets.iter().any(|secret| {
            secret.len() < MIN_SECRET_BYTES
                || secret.len() > MAX_SECRET_BYTES
                || REDACTION_MARKER_TEXT.contains(secret.as_str())
                || secret.contains(REDACTION_MARKER_TEXT)
                || PRIVATE_KEY_MARKER_TEXT.contains(secret.as_str())
                || secret.contains(PRIVATE_KEY_MARKER_TEXT)
                || spans_marker_boundary(secret, REDACTION_MARKER_TEXT)
                || spans_marker_boundary(secret, PRIVATE_KEY_MARKER_TEXT)
        });
        if collides {
            return Self {
                pending: String::new(),
                held_raw: String::new(),
                tail_raw: Vec::new(),
                keep: 0,
                max_secret: 0,
                secrets: Vec::new(),
                refused: true,
            };
        }
        let longest = secrets.iter().map(String::len).max().unwrap_or(0);
        Self {
            pending: String::new(),
            held_raw: String::new(),
            tail_raw: Vec::new(),
            keep: MIN_KEEP.saturating_add(KEEP_SLACK),
            max_secret: longest,
            secrets: secrets.to_vec(),
            refused: false,
        }
    }

    /// Whether the redactor is sealed: either construction refused the
    /// secret set, or it refused itself mid-stream (see
    /// [`MAX_STAGE_A_HOLD`]) — this can turn true mid-stream. The writer
    /// must seal immediately and drop every chunk without redacting or
    /// writing.
    pub(super) fn is_refused(&self) -> bool {
        self.refused
    }

    /// Feeds one raw pump chunk and returns the redacted bytes now safe to
    /// append. The complete-UTF-8 prefix of the held raw tail plus `raw`
    /// is decoded. Stage A joins the decoded text to the RAW holdback and
    /// scrubs known secrets over it, releasing the prefix before a
    /// `max_secret - 1` byte holdback into the stage-B carry (a scrub span
    /// crossing the cut moves the cut to its start, so no value is split).
    /// Stage B applies the pattern pass to the carried redacted text with
    /// holdback `MIN_KEEP + KEEP_SLACK`, moving the cut back to a char
    /// boundary: `full[..cut]` is emitted and `full[cut..]` is carried.
    /// Secrets are never re-scanned after stage A, so marker text in
    /// carried output cannot hide or recreate a value.
    pub(super) fn push(&mut self, raw: &[u8]) -> Vec<u8> {
        if self.refused {
            return Vec::new();
        }
        let mut window = std::mem::take(&mut self.tail_raw);
        window.extend_from_slice(raw);
        let mut text = String::new();
        loop {
            match std::str::from_utf8(&window) {
                Ok(complete) => {
                    text.push_str(complete);
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    // The prefix is valid UTF-8 by Utf8Error's contract.
                    if let Ok(head) = std::str::from_utf8(&window[..valid]) {
                        text.push_str(head);
                    }
                    match error.error_len() {
                        // Genuinely invalid bytes: one replacement char at
                        // this point only, then keep decoding the rest.
                        Some(bad_len) => {
                            text.push('\u{FFFD}');
                            window.drain(..valid + bad_len);
                        }
                        // Incomplete trailing sequence: hold it raw for the
                        // next push instead of lossy-decoding it here.
                        None => {
                            self.tail_raw = window.split_off(valid);
                            break;
                        }
                    }
                }
            }
        }
        // Stage A: secrets, over RAW held text so a value that began in an
        // earlier push still matches whole (nested, partially overlapping
        // and self-overlapping values included).
        let mut window = std::mem::take(&mut self.held_raw);
        window.push_str(&text);
        let spans = secret_spans(&window, &self.secrets);
        let mut safe_end = window
            .len()
            .saturating_sub(self.max_secret.saturating_sub(1));
        while !window.is_char_boundary(safe_end) {
            safe_end -= 1;
        }
        if let Some(&(start, _)) = spans
            .iter()
            .find(|&&(start, end)| start < safe_end && safe_end < end)
        {
            safe_end = start;
        }
        if window.len() - safe_end > MAX_STAGE_A_HOLD {
            // An overlapping match chain would force an unbounded raw hold:
            // seal instead. The writer sees is_refused() and stops storing.
            self.refused = true;
            self.pending = String::new();
            self.held_raw = String::new();
            self.tail_raw = Vec::new();
            return Vec::new();
        }
        let ready = scrub_secrets(&window[..safe_end], &self.secrets);
        self.held_raw = window[safe_end..].to_owned();
        // Stage B: patterns, over the carried already-redacted text.
        let mut carry = std::mem::take(&mut self.pending);
        carry.push_str(&ready);
        let mut full = redact_chunk(&carry, &[]);
        let mut cut = full.len().saturating_sub(self.keep);
        while cut > 0 && !full.is_char_boundary(cut) {
            cut -= 1;
        }
        self.pending = full.split_off(cut);
        full.into_bytes()
    }

    /// Ends the stream: scrubs the RAW holdback and applies the pattern
    /// pass to everything still withheld. An incomplete trailing UTF-8
    /// sequence can never complete after EOF, so it is rendered as U+FFFD
    /// and passes the gate with the rest.
    pub(super) fn finish(&mut self) -> Vec<u8> {
        if self.refused {
            return Vec::new();
        }
        let mut raw = std::mem::take(&mut self.held_raw);
        let tail = std::mem::take(&mut self.tail_raw);
        if !tail.is_empty() {
            raw.push_str(&String::from_utf8_lossy(&tail));
        }
        let scrubbed = scrub_secrets(&raw, &self.secrets);
        let mut carry = std::mem::take(&mut self.pending);
        carry.push_str(&scrubbed);
        redact_chunk(&carry, &[]).into_bytes()
    }

    /// Test-only view of the total holdback size — the stage-B holdback
    /// plus the stage-A raw holdback — so unit tests can place the
    /// emission cut deterministically.
    #[cfg(test)]
    fn holdback(&self) -> usize {
        self.keep + self.max_secret.saturating_sub(1)
    }
}

/// True when `secret` could be completed by text produced next to a redaction
/// marker: a non-empty suffix of `marker` is a prefix of `secret`, or a non-empty
/// prefix of `marker` is a suffix of `secret`. Output containing such a value
/// could then be SYNTHESIZED by the redaction itself ("token=abc X" becoming
/// "token=[redacted] X"), so the set is refused.
fn spans_marker_boundary(secret: &str, marker: &str) -> bool {
    let secret = secret.as_bytes();
    let marker = marker.as_bytes();
    (1..marker.len())
        .any(|k| secret.starts_with(&marker[marker.len() - k..]) || secret.ends_with(&marker[..k]))
}

#[cfg(test)]
#[path = "output_redact_tests.rs"]
mod output_redact_tests;

#[cfg(test)]
#[path = "output_redact_overlap_tests.rs"]
mod output_redact_overlap_tests;

#[cfg(test)]
#[path = "output_redact_split_tests.rs"]
mod output_redact_split_tests;
