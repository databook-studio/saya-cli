use crate::SessionState;
use reedline::{Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus};
use std::borrow::Cow;

/// Builds a compact one-line status header shown with the interactive prompt:
/// the active profile, any included databases, the provider/model, the approval
/// mode, and the cloud data-sharing (privacy) state.
pub(crate) fn status_line(state: &SessionState) -> String {
    let profile = state.profile.as_deref().unwrap_or("(no profile)");
    let included = if state.included_profiles.is_empty() {
        String::new()
    } else {
        format!(
            " {}",
            state
                .included_profiles
                .iter()
                .map(|profile| format!("+{profile}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    let privacy = if state.allow_data_sharing {
        "privacy:on"
    } else {
        "privacy:off"
    };
    format!(
        "[{profile}{included}] {}/{} approval:{} {privacy}",
        state.provider, state.model, state.approval_mode
    )
}

/// Reedline prompt for interactive sessions: a single-line status header
/// followed by the `saya> ` input marker.
pub(crate) struct SayaPrompt {
    header: String,
}

impl SayaPrompt {
    /// Builds a prompt from the current session status.
    pub(crate) fn new(state: &SessionState) -> Self {
        Self {
            header: status_line(state),
        }
    }
}

impl Prompt for SayaPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Owned(format!("{} ", self.header))
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("saya> ")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("... ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({}reverse-search: {}) ",
            prefix, history_search.term
        ))
    }
}
