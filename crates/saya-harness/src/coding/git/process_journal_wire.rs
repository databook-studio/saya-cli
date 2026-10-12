//! Journal line wire format: the durable `JournalReceipt` record and the
//! encode/decode sides of the JSONL journal.
use super::super::{
    command::GitOperation,
    process_observer::{GitProcessEffect, GitProcessOutcome, GitProcessReceipt},
};

// This module is loaded through `#[path = "process_journal_wire.rs"]` from
// `process_journal.rs`, so it does not own a subdirectory: the child module
// files need explicit paths to live in `process_journal_wire/`.
#[path = "process_journal_wire/decode.rs"]
mod decode;

#[path = "process_journal_wire/encode.rs"]
mod encode;

pub(in crate::coding::git) use decode::decode_line;
pub(in crate::coding::git) use encode::encode_receipt;
pub(super) use encode::{INCOMPLETE_LINE, encode_session_close, encode_session_start};

const VERSION: u32 = 1;
const MAX_LINE_BYTES: usize = 512;
pub(super) const MAX_LINE: usize = MAX_LINE_BYTES;

pub(in crate::coding::git) struct JournalReceipt {
    pub(super) version: u32,
    pub(super) service_identity: String,
    pub(super) sequence: u64,
    pub(super) request_kind: &'static str,
    pub(super) operation: GitOperation,
    pub(super) pid: Option<u32>,
    pub(super) exit_code: Option<i32>,
    pub(super) signal: Option<i32>,
    pub(super) outcome: GitProcessOutcome,
    pub(super) effect: GitProcessEffect,
    pub(super) killed_orphans: Option<bool>,
    pub(super) duration_ms: u64,
    pub(super) stdout_bytes: u64,
    pub(super) stderr_bytes: u64,
    pub(super) stdout_truncated: bool,
    pub(super) stderr_truncated: bool,
}

pub(in crate::coding::git) enum JournalLine {
    Receipt(JournalReceipt),
    SessionStart(String),
    SessionClose(String),
}
