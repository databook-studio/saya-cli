//! The one message a busy session holds.
//!
//! A turn runs one at a time, so a message sent mid-turn waits here instead of
//! being refused. The queue is one deep: the operator's last word wins, and a
//! message it displaces is reported rather than silently dropped.

/// A queue one message deep.
#[derive(Debug, Default)]
pub(crate) struct SendQueue {
    pending: Option<String>,
}

/// What became of a message offered to the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Enqueued {
    /// The queue was empty and now holds the message.
    Held,
    /// The queue already held a message, and this one displaced it.
    Replaced,
}

impl SendQueue {
    /// An empty queue.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Offers one message. A queue that already holds a message keeps the
    /// newer one and says so, so the surface hears that a message it showed as
    /// waiting will not run.
    pub(crate) fn offer(&mut self, message: String) -> Enqueued {
        // A replaced message is dropped here, not kept beside the newer one: two
        // waiting messages would make the session's depth two, and the operator's
        // last word is the one they meant.
        match self.pending.replace(message) {
            Some(_) => Enqueued::Replaced,
            None => Enqueued::Held,
        }
    }

    /// Takes the waited message, if one is waiting.
    pub(crate) fn take(&mut self) -> Option<String> {
        self.pending.take()
    }

    /// Whether nothing is waiting.
    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_none()
    }
}
