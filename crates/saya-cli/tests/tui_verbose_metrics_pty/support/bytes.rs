/// First (leftmost) position of `needle` in `haystack`.
fn position_of(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Last (rightmost) position of `needle` in `haystack`.
fn rposition_of(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// How many times `needle` occurs in `haystack`.
fn count_of(haystack: &[u8], needle: &[u8]) -> usize {
    if haystack.len() < needle.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

/// The readable tail of the raw stream, for failure messages.
fn raw_tail(raw: &[u8]) -> String {
    let start = raw.len().saturating_sub(4000);
    String::from_utf8_lossy(&raw[start..]).into_owned()
}
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
