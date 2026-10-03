use saya_types::RunEvent;

use super::{EngineEventSink, EngineSinkError};

impl EngineEventSink {
    /// Journals the download wallet's consumed level when it has grown past
    /// the level the journal holds — the durable record a resume seeds the
    /// wallet from, the same role the usage record plays for the token
    /// ceiling. The wallet's arithmetic is first-party (the claim counter is
    /// ours, never a provider's report), so the event carries a definite
    /// figure — never absent, never a fabricated zero for spend that was
    /// never claimed. The recorded levels are monotone, and the journal
    /// holds the wallet's spend as far as the record goes: the bytes a
    /// download claims between two ticks are journaled at the next one, so
    /// what a crash can leave unrecorded is the in-flight attempt's claims —
    /// the same residual the usage path accepts, which a restarted download
    /// re-pays by claiming its remainder, never invents.
    ///
    /// A failed append is held as a diagnostic and the level is not
    /// advanced, so the next tick retries it; emit has no error channel,
    /// and the next transition's write fails loudly the same way.
    pub(super) fn journal_downloads(&self) {
        let Some(budget) = &self.download_budget else {
            return;
        };
        let level = budget.consumed();
        let mut journaled = self
            .journaled_downloads
            .lock()
            .expect("engine sink download journal lock");
        if level <= *journaled {
            return;
        }
        let event = RunEvent::DownloadedBytes { bytes: level };
        match self.journal.append(&event) {
            Ok(()) => *journaled = level,
            Err(source) => self.hold_diagnostic(EngineSinkError::Journal { source }),
        }
    }
}

impl EngineEventSink {
    /// Journals one call's usage as the durable record `saya run show` and
    /// `saya run log` read back. The event is labelled with the endpoint the
    /// episode calls — today always the single orchestrator endpoint, the
    /// same one bucket the token ceiling sums — and carries each figure only
    /// when the provider reported it: an unreported cache figure journals
    /// absent (unknown), never zero. The journal is the durable authority;
    /// the store's usage mirror is a reconciliation concern, not this one.
    ///
    /// A failed append is held as a diagnostic: emit has no error channel,
    /// and the next transition's write fails loudly the same way.
    pub(super) fn journal_usage(&self, usage: &saya_agent::TokenUsage) {
        let event = RunEvent::Usage {
            endpoint: crate::endpoints::ORCHESTRATOR_ROLE.to_string(),
            tokens: Some(usage.input_tokens.saturating_add(usage.output_tokens)),
            turns: None,
            tool_calls: None,
            cached_input_tokens: usage.cached_input_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens,
        };
        if let Err(source) = self.journal.append(&event) {
            self.hold_diagnostic(EngineSinkError::Journal { source });
        }
    }
}
