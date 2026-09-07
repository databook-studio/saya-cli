use crate::SessionState;

/// Builds a compact one-line status header shown with the interactive prompt:
/// the active profile, any included databases, the provider/model, the approval
/// mode, and the cloud data-sharing state. The sharing segment names what is
/// happening (`sharing:on` = row values are sent to the provider), not a
/// protection claim — `allow_data_sharing == true` means data *is* shared, so
/// the label must not read as protection.
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
    let sharing = if state.allow_data_sharing {
        "sharing:on"
    } else {
        "sharing:off"
    };
    format!(
        "[{profile}{included}] {}/{} approval:{} {sharing}",
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
    /// Mirrors `status_line`'s mapping: `allow_data_sharing` => `sharing:on`.
    pub(crate) sharing_on: bool,
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
        sharing_on: state.allow_data_sharing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a session with just the fields the status line reads.
    fn session_with_sharing(allow_data_sharing: bool) -> SessionState {
        let mut state = SessionState::new("s1", Some(String::from("analytics")), "qwen");
        state.provider = "ollama".into();
        state.approval_mode = "read-only".into();
        state.allow_data_sharing = allow_data_sharing;
        state
    }

    /// The status segment names what is *happening* (`sharing:on` = row values are
    /// sent to the model provider), not a protection claim. The old label
    /// (`privacy:on`) read as protection while `allow_data_sharing == true` meant the
    /// opposite — data *was* being shared — so a user concluded the reverse of the
    /// truth. This pins the corrected mapping: the shared state is labelled
    /// `sharing:on`, the private state `sharing:off`.
    #[test]
    fn status_line_names_sharing_not_protection() {
        let sharing = status_line(&session_with_sharing(true));
        assert!(
            sharing.contains("sharing:on") && !sharing.contains("privacy:"),
            "allow_data_sharing=true must read sharing:on (data is being sent), got: {sharing}"
        );

        let private = status_line(&session_with_sharing(false));
        assert!(
            private.contains("sharing:off") && !private.contains("privacy:"),
            "allow_data_sharing=false must read sharing:off (data stays local), got: {private}"
        );
    }

    /// The TUI status bar mirrors the one-line header, so the same `allow_data_sharing`
    /// state yields the same label on both surfaces. This is the anti-drift check: the
    /// structured `StatusView` and the rendered `status_line` agree on which state is
    /// "on".
    #[test]
    fn status_segments_mirror_status_line_polarity() {
        for allow in [true, false] {
            let state = session_with_sharing(allow);
            let line = status_line(&state);
            let view = status_segments(&state);
            let expected = if allow { "sharing:on" } else { "sharing:off" };
            assert_eq!(
                view.sharing_on, allow,
                "StatusView.sharing_on must equal allow_data_sharing"
            );
            assert!(
                line.contains(expected),
                "status_line ({line}) must match StatusView ({expected}) for allow={allow}"
            );
        }
    }
}
