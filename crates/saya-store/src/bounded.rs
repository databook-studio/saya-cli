//! A fail-fast byte ceiling for JSON serialization: a [`Write`] adapter
//! that counts bytes and errors at the bound, so `serde_json::to_writer`
//! refuses an oversized value *during* serialization instead of after a
//! full in-memory materialization. Shared by the session save and the schema
//! cache write — both serialize untrusted-shaped data under an aggregate cap.

use std::io::{self, Write};

/// The writer-side refusal: the value grew past the stated byte ceiling.
#[derive(Debug)]
pub(crate) struct OverBound {
    pub(crate) max: usize,
}

impl std::fmt::Display for OverBound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "value exceeds the {}-byte bound", self.max)
    }
}

impl std::error::Error for OverBound {}

/// A [`Write`] sink that accepts at most `max` bytes, then fails. The peak
/// retained allocation is bounded by `max` plus one write call's chunk —
// trusted code must still size the chunk sanely — but never by the value's
// full serialized size: refusal happens mid-stream, not after `to_vec`.
pub(crate) struct BoundedWriter<W: Write> {
    inner: W,
    written: usize,
    max: usize,
    /// High-water mark of bytes retained in `inner`, observed for tests.
    peak: usize,
}

impl<W: Write> BoundedWriter<W> {
    pub(crate) fn new(inner: W, max: usize) -> Self {
        Self {
            inner,
            written: 0,
            max,
            peak: 0,
        }
    }

    /// High-water mark of retained bytes, observed by tests to prove the
    /// refusal happens mid-stream. Test-only: production code consumes the
    /// writer and never inspects it.
    #[cfg(test)]
    pub(crate) fn peak(&self) -> usize {
        self.peak
    }

    #[cfg(test)]
    pub(crate) fn written(&self) -> usize {
        self.written
    }

    /// Consumes the writer, returning what was retained. Only reachable on
    /// the success path — oversized values error out of `to_writer` before
    /// this is ever called — so the returned allocation never exceeds `max`.
    pub(crate) fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let next = self.written.saturating_add(buf.len());
        if next > self.max {
            return Err(io::Error::new(
                io::ErrorKind::QuotaExceeded,
                OverBound { max: self.max },
            ));
        }
        let wrote = self.inner.write(buf)?;
        self.written += wrote;
        self.peak = self.peak.max(self.written);
        Ok(wrote)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_mid_stream_without_retaining_the_full_value() {
        let max = 16;
        let mut capped = BoundedWriter::new(Vec::new(), max);
        let big: Vec<u64> = (0..1_000).collect();
        let result = serde_json::to_writer(&mut capped, &big);
        assert!(result.is_err(), "an oversized value must refuse mid-stream");
        assert!(
            capped.peak() <= max,
            "peak retained bytes stay within the bound: {}",
            capped.peak()
        );
    }

    #[test]
    fn at_the_bound_still_serializes() {
        let mut capped = BoundedWriter::new(Vec::new(), 5);
        serde_json::to_writer(&mut capped, &vec![1u8, 2u8]).expect("at-bound passes");
        assert_eq!(capped.written(), 5);
    }
}
