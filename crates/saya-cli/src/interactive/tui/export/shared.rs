//! Shared row shape plus the bounded in-memory sink both encoders write
//! through.

pub(super) const EXPORT_TOO_LARGE: &str = "export larger than 32 MiB";

pub(super) fn normalize_row(row: &serde_json::Value, col_count: usize) -> Vec<serde_json::Value> {
    let mut cells = match row {
        serde_json::Value::Array(arr) => arr.clone(),
        scalar => vec![scalar.clone()],
    };
    cells.resize(col_count, serde_json::Value::Null);
    cells
}

/// In-memory sink with a hard ceiling checked while writing: the write that
/// would cross the ceiling is refused and everything after it is discarded,
/// so an oversize export never fully materialises. The refusal is reported
/// as [`EXPORT_TOO_LARGE`], the one oversize message both encoders share.
pub(super) struct BoundedWriter {
    buf: Vec<u8>,
    ceiling: usize,
    exceeded: bool,
}

impl BoundedWriter {
    pub(super) fn new(ceiling: usize) -> Self {
        Self {
            buf: Vec::new(),
            ceiling,
            exceeded: false,
        }
    }

    /// Whether any write was refused; the caller reports
    /// [`EXPORT_TOO_LARGE`] and discards the buffer.
    pub(super) fn exceeded(&self) -> bool {
        self.exceeded
    }

    /// The written bytes; meaningful only when nothing was refused.
    pub(super) fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    pub(super) fn write_str(&mut self, text: &str) -> Result<(), String> {
        if self.buf.len() + text.len() > self.ceiling {
            self.exceeded = true;
            return Err(EXPORT_TOO_LARGE.into());
        }
        self.buf.extend_from_slice(text.as_bytes());
        Ok(())
    }
}

impl std::io::Write for BoundedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() + buf.len() > self.ceiling {
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                EXPORT_TOO_LARGE,
            ));
        }
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ceiling is exact: a write landing exactly on it is allowed, the
    /// first byte past it is refused and sets the flag.
    #[test]
    fn the_ceiling_is_checked_while_writing() {
        let mut out = BoundedWriter::new(4);
        out.write_str("abcd").expect("exactly at the ceiling fits");
        assert!(!out.exceeded());
        assert!(out.write_str("e").is_err(), "one byte past is refused");
        assert!(out.exceeded());
        assert_eq!(out.into_inner(), b"abcd", "refused bytes are not appended");

        let mut out = BoundedWriter::new(4);
        out.write_str("abcde")
            .expect_err("a write crossing the ceiling is refused");
        assert!(out.exceeded());
        assert!(
            out.into_inner().is_empty(),
            "nothing of a refused write lands"
        );
    }
}
