//! Driving and observing the private PTY session under test.

use super::{INPUT_LIMIT, IO_WAIT, OUTPUT_LIMIT, Session};
use std::io::Write;
use std::time::{Duration, Instant};

impl Session {
    pub fn send(&mut self, bytes: &[u8]) {
        assert!(bytes.len() <= INPUT_LIMIT, "one PTY input is bounded");
        let writer = self.writer.as_mut().expect("PTY writer remains live");
        writer.write_all(bytes).unwrap();
        writer.flush().unwrap();
    }

    pub fn wait_for(&mut self, needle: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.screen_text().contains(needle) {
                return;
            }
            self.pump();
        }
        panic!(
            "PTY screen did not show {needle:?}; screen:\n{}\nraw tail:\n{}",
            self.screen_text(),
            self.tail()
        );
    }

    pub fn checkpoint(&self) -> usize {
        self.raw.len()
    }

    pub fn wait_for_after(&mut self, checkpoint: usize, needle: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self
                .raw
                .get(checkpoint..)
                .is_some_and(|bytes| String::from_utf8_lossy(bytes).contains(needle))
            {
                return;
            }
            self.pump();
        }
        panic!(
            "PTY screen did not show {needle:?} after checkpoint; screen:\n{}\nraw tail:\n{}",
            self.screen_text(),
            self.tail()
        );
    }

    pub fn pause_ticket_after(&mut self, checkpoint: usize, timeout: Duration) -> u64 {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let text = String::from_utf8_lossy(self.raw.get(checkpoint..).unwrap_or_default());
            if let Some(start) = text.find("pause_requested\":") {
                let digits = &text[start + "pause_requested\":".len()..];
                let end = digits
                    .find(|character: char| !character.is_ascii_digit())
                    .unwrap_or(digits.len());
                if end > 0 {
                    return digits[..end].parse().expect("pause ticket is numeric");
                }
            }
            self.pump();
        }
        panic!(
            "pause ticket did not appear after checkpoint; screen:\n{}\nraw tail:\n{}",
            self.screen_text(),
            self.tail()
        );
    }

    pub fn wait_for_pause_ack(
        &mut self,
        checkpoint: usize,
        ticket: u64,
        target: &str,
        timeout: Duration,
    ) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let text = String::from_utf8_lossy(self.raw.get(checkpoint..).unwrap_or_default());
            let mut cursor = 0;
            while let Some(offset) = text[cursor..].find('{') {
                let start = cursor + offset;
                let Some(end) = text[start..].find('}') else {
                    break;
                };
                let object = &text[start..=start + end];
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(object) {
                    let matches = value
                        .get("pause_requested")
                        .and_then(|value| value.as_u64())
                        == Some(ticket)
                        && value
                            .get("pause_acknowledged")
                            .and_then(|value| value.as_bool())
                            == Some(true)
                        && value.get("target").and_then(|value| value.as_str()) == Some(target);
                    if matches {
                        return;
                    }
                }
                cursor = start + 1;
            }
            self.pump();
        }
        panic!(
            "matching pause acknowledgement did not appear on PTY screen:\n{}\nraw tail:\n{}",
            self.screen_text(),
            self.tail()
        );
    }

    pub(super) fn pump(&mut self) {
        if let Ok(chunk) = self.output.recv_timeout(IO_WAIT) {
            self.raw.extend_from_slice(&chunk);
            self.parser.process(&chunk);
            assert!(self.raw.len() <= OUTPUT_LIMIT, "PTY transcript is bounded");
        }
    }

    fn tail(&self) -> String {
        let start = self.raw.len().saturating_sub(4096);
        String::from_utf8_lossy(&self.raw[start..]).into_owned()
    }
}
