//! The capture outcome types the file tools carry between the prepared
//! snapshot and the commit: what `begin` decided before any byte was
//! written, and the handle to the record that must be finalized, dropped,
//! or kept as `Uncertain` afterwards.

/// One edit's capture outcome, decided before any byte is written.
#[derive(Debug)]
pub(in crate::agent::tools) enum Capture {
    /// Recovery is off: no snapshot, no cleanup, no result field.
    Off,
    /// A `Prepared` record exists; the edit must finalize or settle it.
    Recorded(Pending),
    /// Admission screening refused the content: the edit proceeds without
    /// a checkpoint and the result says so.
    Unavailable(&'static str),
    /// Capture refused and the edit must not run.
    Refused(String),
}

impl Capture {
    /// The model-safe refusal to raise before any write, if capture
    /// refused.
    pub(in crate::agent::tools) fn refused_text(&self) -> Option<&str> {
        match self {
            Capture::Refused(text) => Some(text),
            _ => None,
        }
    }
}

/// One prepared checkpoint awaiting the edit's outcome.
#[derive(Debug)]
pub(in crate::agent::tools) struct Pending {
    pub(super) id: String,
}
