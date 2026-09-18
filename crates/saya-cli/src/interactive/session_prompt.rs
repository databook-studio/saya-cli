use crate::SessionState;

/// Builds a compact one-line status header shown with the interactive prompt:
/// the active profile, any included databases, the provider/model, the
/// approval mode, the workspace root, the host lane, and the cloud
/// data-sharing state. The
/// sharing segment names what is happening (`sharing:on` = row values are
/// sent to the provider), not a protection claim — `allow_data_sharing ==
/// true` means data *is* shared, so the label must not read as protection.
/// The workspace segment names the tree the session can touch: the bound
/// canonical root, or `ws:none` — the no-root shape where the write-shaped
/// tools are absent and workspace reads refuse. The host segment names the
/// lane: `host:unsandboxed` where the host-command lane composed (host
/// commands run unsandboxed — as the user, their network, their filesystem),
/// `host:off` where it did not, plus `deny:<names>` where the session's deny
/// list is non-empty. This is where the binding
/// is visible at every moment it matters, including on a resume whose cwd
/// differs from the launch one.
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
    let workspace = match state.workspace_root.as_deref() {
        Some(root) => format!("ws:{root}"),
        None => "ws:unbound".to_string(),
    };
    let mut host = if state.host_composed {
        "host:unsandboxed".to_string()
    } else {
        "host:off".to_string()
    };
    if !state.denied_programs.is_empty() {
        host.push_str(&format!(" deny:{}", state.denied_programs.join(",")));
    }
    // The task segment names the done count while anything is tracked, and
    // is absent entirely on an empty list — the other conditional segments'
    // shape, so an untracked session reads exactly as before.
    let tasks = super::session_tasks_view::tasks_summary(&state.task_list)
        .map(|summary| format!(" {summary}"))
        .unwrap_or_default();
    format!(
        "[{profile}{included}] {}/{} approval:{} mode:{}{tasks} {workspace} {host} {sharing}",
        state.provider, state.model, state.approval_mode, state.agent_mode
    )
}

/// Structured status for the TUI status bar, so each segment can be coloured.
pub(crate) struct StatusView {
    pub(crate) profile: String,
    pub(crate) included: Vec<String>,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) approval_mode: String,
    /// The session's task posture — mirrors `status_line`'s `mode:` segment,
    /// so the TUI bar and the headless header cannot drift.
    pub(crate) agent_mode: String,
    /// The pinned workspace root, when one binds — mirrors `status_line`'s
    /// segment; `None` renders the no-root shape.
    pub(crate) workspace_root: Option<String>,
    /// Mirrors `status_line`'s mapping: `allow_data_sharing` => `sharing:on`.
    pub(crate) sharing_on: bool,
    /// Whether the host-command lane composed — mirrors `status_line`'s
    /// `host:` segment.
    pub(crate) host_composed: bool,
    /// The session's deny list — mirrors `status_line`'s `deny:` listing.
    pub(crate) denied_programs: Vec<String>,
    /// The session task list's done count — mirrors `status_line`'s
    /// `tasks:` segment; `None` renders no segment, the empty-list shape.
    pub(crate) task_summary: Option<String>,
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
        agent_mode: state.agent_mode.clone(),
        workspace_root: state.workspace_root.clone(),
        sharing_on: state.allow_data_sharing,
        host_composed: state.host_composed,
        denied_programs: state.denied_programs.clone(),
        task_summary: super::session_tasks_view::tasks_summary(&state.task_list),
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

    /// The workspace segment names the bound root — the binding is visible at
    /// every moment it matters, including on a resume whose cwd differs — and
    /// the no-root shape is named as unbound, not hidden.
    #[test]
    fn status_line_names_the_workspace_binding() {
        let mut bound = session_with_sharing(false);
        bound.workspace_root = Some("/projects/saya".into());
        let line = status_line(&bound);
        assert!(
            line.contains("ws:/projects/saya"),
            "the status header names the pinned root: {line}"
        );
        let segments = status_segments(&bound);
        assert_eq!(
            segments.workspace_root.as_deref(),
            Some("/projects/saya"),
            "the TUI view mirrors the header"
        );

        let unbound = status_line(&session_with_sharing(false));
        assert!(
            unbound.contains("ws:unbound") && !unbound.contains("ws:/"),
            "no root reads as unbound: {unbound}"
        );
        assert!(
            status_segments(&session_with_sharing(false))
                .workspace_root
                .is_none(),
            "the TUI view mirrors the no-root shape"
        );
    }

    /// The header gains a `mode:` segment beside `approval:`: build by
    /// default, plan after `/mode plan`. The structured `StatusView` the TUI
    /// reads carries the same word — the line loop and the TUI cannot drift.
    #[test]
    fn status_line_and_view_carry_the_mode_segment() {
        use crate::slash::SlashCommand;
        let mut state = session_with_sharing(false);
        let line = status_line(&state);
        assert!(
            line.contains("mode:build"),
            "the default posture reads mode:build: {line}"
        );
        assert!(
            line.contains("approval:"),
            "the mode segment sits beside approval:: {line}"
        );
        assert_eq!(
            status_segments(&state).agent_mode,
            "build",
            "the TUI view mirrors the header"
        );

        state.apply(SlashCommand::Mode(Some(saya_agent::AgentMode::Plan)), &[]);
        let planned = status_line(&state);
        assert!(
            planned.contains("mode:plan"),
            "after /mode plan the header reads mode:plan: {planned}"
        );
        assert_eq!(
            status_segments(&state).agent_mode,
            "plan",
            "the TUI view follows the switch"
        );
    }
}
