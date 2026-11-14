//! The one-call repeat detector: the same call returning the same result in a
//! row. It counts, never blocks, and speaks exactly once before the stop.

/// The one-call repeat detector: the same call returning the same result in a
/// row. It counts, never blocks, and speaks exactly once before the stop.
#[derive(Debug, Default)]
pub(super) struct RepeatDetector {
    last: Option<(String, u64)>,
    count: u32,
    noted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RepeatOutcome {
    None,
    Note,
    Stop,
}

impl RepeatDetector {
    pub(super) fn observe(&mut self, key: String, digest: u64, limit: u32) -> RepeatOutcome {
        if self.last.as_ref() == Some(&(key.clone(), digest)) {
            self.count = self.count.saturating_add(1);
        } else {
            self.last = Some((key, digest));
            self.count = 1;
            self.noted = false;
        }
        if self.count == limit && !self.noted {
            self.noted = true;
            return RepeatOutcome::Note;
        }
        if self.count > limit {
            return RepeatOutcome::Stop;
        }
        RepeatOutcome::None
    }
}
