//! The two figures the status row shows about the session's spend and headroom:
//! how full the last request left the model's window, and what the session has
//! cost as far as the provider said.
//!
//! Both are conditional by construction. A window the launch does not know is
//! `None`, and no meter appears; a provider that reports no cost leaves `None`,
//! and no money appears. A reported zero is not absence: a session that has
//! spent nothing yet and was told so reads `$0`.

/// What the row knows about the last request and the session's spend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Meters {
    /// Input tokens the last answering request sent, when a call reported them.
    pub(crate) input_tokens: Option<u64>,
    /// The model's context window in tokens, when the launch knows it.
    pub(crate) window_tokens: Option<u64>,
    /// The session's provider-reported cost so far, in micro-USD. `None` when no
    /// call reported one — unknown, never a reported zero.
    pub(crate) cost_micros: Option<u64>,
}

impl Meters {
    /// The share of the window the last request used, when both are known.
    pub(crate) fn context_percent(self) -> Option<u64> {
        saya_agent::context_utilisation_percent(self.input_tokens, self.window_tokens)
    }

    /// The one segment the row shows, or `None` when it has nothing to say: the
    /// figures in the order they matter, joined by the row's own separator.
    pub(crate) fn segment(self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(percent) = self.context_percent() {
            parts.push(format!("ctx {percent}%"));
        }
        if let Some(micros) = self.cost_micros {
            parts.push(cost_text(micros));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// A reported cost, as money — the session's own reading of the figure, so the
/// row and `/status` cannot word it differently.
pub(crate) fn cost_text(micros: u64) -> String {
    crate::interactive::cost_text(micros)
}

#[cfg(test)]
#[path = "meters_tests.rs"]
mod tests;
