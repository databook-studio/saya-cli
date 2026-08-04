use crate::SessionState;

/// Builds a compact one-line status header shown above the interactive prompt:
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
