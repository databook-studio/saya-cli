//! Every bound the MCP client applies, in one place.
//!
//! A third-party server is untrusted input in the strongest sense: it chooses
//! how many tools it advertises, how long their descriptions are, how large
//! their schemas are, how big a frame it sends, and how long it takes to
//! answer. None of those may be unbounded, and each cap here is named so the
//! refusal that fires can say which one it was.

use std::time::Duration;

/// The MCP revision this client speaks. Sent in `initialize`; the server's
/// answer is accepted whatever it states, because the three requests this
/// client makes (`initialize`, `tools/list`, `tools/call`) have carried the
/// same shape across every revision published so far.
pub(crate) const PROTOCOL_VERSION: &str = "2024-11-05";

/// How many tools one server may contribute. A server that advertises more is
/// cut off here and the cut is reported: the advertisement is part of every
/// request, so its size is a cost paid on every turn.
pub(crate) const MAX_TOOLS_PER_SERVER: usize = 64;

/// One inbound frame's byte ceiling. A server that sends a longer line has
/// that line discarded and the exchange fails — the reader never buffers more
/// than this, whatever the server writes.
pub(crate) const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// How many `tools/list` pages one server's listing may take. The cursor loop
/// is bounded so a server that keeps answering with a fresh cursor cannot
/// hold startup open forever.
pub(crate) const MAX_LIST_PAGES: usize = 8;

/// How many frames may be skipped while waiting for one reply: a server is
/// free to send notifications, but a server that only ever sends them must
/// not spin the reader.
pub(crate) const MAX_SKIPPED_FRAMES: usize = 64;

/// One tool description's character ceiling. The description is shown to the
/// model verbatim, so it is truncated rather than refused: a tool with a long
/// description is still a usable tool.
pub(crate) const MAX_TOOL_DESCRIPTION_CHARS: usize = 1_024;

/// One tool's schema byte ceiling. A schema is the tool's calling contract, so
/// a tool whose schema does not fit is **dropped** rather than truncated — a
/// half-schema would be a contract nobody stated.
pub(crate) const MAX_TOOL_SCHEMA_BYTES: usize = 16 * 1024;

/// What one call may return to the model, after redaction. The result is one
/// tool message, so it carries the same kind of bound a `shell` call's output
/// does.
pub(crate) const MAX_RESULT_BYTES: usize = 64 * 1024;

/// What one call may send: the model's arguments, serialised. Bounded so a
/// runaway argument object cannot be handed to a third-party program.
pub(crate) const MAX_ARGUMENT_BYTES: usize = 64 * 1024;

/// How many bytes of a server's stderr are kept for diagnostics. The stream is
/// drained whatever its size — a server whose stderr pipe fills would block —
/// and only this tail is retained for the report.
pub(crate) const MAX_STDERR_BYTES: usize = 8 * 1024;

/// The whole connection pass's ceiling, across every configured server. Each
/// server also has its own `startup_timeout_ms`; whichever is smaller wins, so
/// a config with eight slow servers still cannot hold a session's start longer
/// than this.
pub(crate) const MAX_CONNECT_TOTAL: Duration = Duration::from_secs(30);
