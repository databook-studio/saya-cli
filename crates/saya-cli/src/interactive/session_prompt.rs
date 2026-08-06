use crate::SessionState;

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

/// Structured status for the TUI status bar, so each segment can be coloured.
pub(crate) struct StatusView {
    pub(crate) profile: String,
    pub(crate) included: Vec<String>,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) approval_mode: String,
    /// Mirrors `status_line`'s mapping: `allow_data_sharing` => "privacy:on".
    pub(crate) privacy_on: bool,
}

/// Returns structured status bar segments for the active session state.
pub(crate) fn status_segments(state: &SessionState) -> StatusView {
    StatusView {
        profile: state
            .profile
            .clone()
            .unwrap_or_else(|| "(no profile)".to_string()),
        included: state.included_profiles.clone(),
        provider: state.provider.clone(),
        model: state.model.clone(),
        approval_mode: state.approval_mode.clone(),
        privacy_on: state.allow_data_sharing,
    }
}
