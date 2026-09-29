//! The bounded line gate (ADR 0008, invariant 1): the state machine every
//! inbound stdin byte passes through before rmcp ever sees it, and the
//! per-line decode decision.
//!
//! rmcp's stdio transport reads newline-delimited frames with no size cap of
//! its own — a client could push an unbounded line into memory. This gate
//! assembles lines itself and hands the transport only completed lines that
//! fit [`super::policy::MAX_REQUEST_BYTES`]; a longer line is discarded up to
//! its newline and answered with a JSON-RPC error (-32600, "request too
//! large") by [`super::transport`]. Memory never exceeds the cap plus one
//! fixed read chunk, whatever the client sends.

use rmcp::{RoleServer, model::ErrorData, service::RxJsonRpcMessage};

type Inbound = RxJsonRpcMessage<RoleServer>;

/// What one accepted line asks the transport to do, matching rmcp's own
/// codec semantics: deliver the message, answer well-formed-but-wrong-shape
/// JSON with an invalid-request error, or ignore an unparseable line.
pub(crate) enum LineAction {
    Deliver(Box<Inbound>),
    Answer(ErrorData),
    Ignore,
}

/// Parses one accepted line exactly like rmcp's codec would: an unparseable
/// line is ignored (there is no id to correlate a reply with), and
/// well-formed JSON that does not match the message shape is answered with
/// an invalid-request error.
pub(crate) fn parse_inbound(line: &[u8]) -> LineAction {
    match serde_json::from_slice::<Inbound>(line) {
        Ok(message) => LineAction::Deliver(Box::new(message)),
        Err(error) => match error.classify() {
            serde_json::error::Category::Data | serde_json::error::Category::Io => {
                LineAction::Answer(ErrorData::invalid_request("Invalid request", None))
            }
            serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
                LineAction::Ignore
            }
        },
    }
}

/// The oversized-line refusal: an error response with a null id — the line
/// is discarded unread, so the id it may have carried is unknown, and
/// JSON-RPC's rule for an undetectable id is `null`.
pub(crate) fn oversized_line_error() -> ErrorData {
    ErrorData::new(
        rmcp::model::ErrorCode::INVALID_REQUEST,
        "request too large",
        None,
    )
}

/// Assembles stdin bytes into lines under a byte cap. `absorb` calls
/// `deliver` once per completed accepted line (without the newline) and
/// `refuse` once per discarded oversized line, at the moment its newline
/// arrives. A line still open at end-of-input is never delivered and never
/// refused — there is no complete request to answer.
pub(crate) struct LineGate {
    line: Vec<u8>,
    discarding: bool,
    cap: usize,
}

impl LineGate {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            line: Vec::new(),
            discarding: false,
            cap,
        }
    }

    /// Feed one chunk of raw input. `deliver` gets each completed accepted
    /// line without its newline; `refuse` fires once per discarded line.
    pub(crate) fn absorb(
        &mut self,
        chunk: &[u8],
        deliver: &mut dyn FnMut(&[u8]),
        refuse: &mut dyn FnMut(),
    ) {
        let mut cursor = 0;
        while cursor < chunk.len() {
            match chunk[cursor..].iter().position(|byte| *byte == b'\n') {
                Some(offset) => {
                    let end = cursor + offset;
                    if self.discarding {
                        // The oversized line ends here: drop its tail and
                        // answer, then resume for the next line.
                        self.discarding = false;
                        refuse();
                    } else if self.line.len() + (end - cursor) <= self.cap {
                        self.line.extend_from_slice(&chunk[cursor..end]);
                        deliver(&self.line);
                    } else {
                        // The line completes in this chunk but overflowed.
                        refuse();
                    }
                    self.line.clear();
                    cursor = end + 1;
                }
                None => {
                    if !self.discarding {
                        if self.line.len() + (chunk.len() - cursor) <= self.cap {
                            self.line.extend_from_slice(&chunk[cursor..]);
                        } else {
                            self.line.clear();
                            self.discarding = true;
                        }
                    }
                    cursor = chunk.len();
                }
            }
        }
    }

    /// End of input: a line still being assembled is dropped (a trailing
    /// fragment is not a request), and the discard state ends with the
    /// stream.
    pub(crate) fn finish(&mut self) {
        self.line.clear();
        self.discarding = false;
    }
}

#[cfg(test)]
#[path = "line_gate_tests.rs"]
mod tests;
