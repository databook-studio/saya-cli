use serde::Serialize;
use std::io;

pub(super) enum CountError {
    Exceeded,
    Serialization,
}

pub(super) fn measure<T: Serialize>(value: &T, cap: usize) -> Result<usize, CountError> {
    let mut writer = CountingWriter {
        bytes: 0,
        cap,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.exceeded {
        Err(CountError::Exceeded)
    } else if result.is_err() {
        Err(CountError::Serialization)
    } else {
        Ok(writer.bytes)
    }
}

struct CountingWriter {
    bytes: usize,
    cap: usize,
    exceeded: bool,
}

impl io::Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(next) = self.bytes.checked_add(bytes.len()) else {
            self.exceeded = true;
            return Err(io::Error::other("request projection limit exceeded"));
        };
        if next > self.cap {
            self.exceeded = true;
            return Err(io::Error::other("request projection limit exceeded"));
        }
        self.bytes = next;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
