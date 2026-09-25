use super::*;

fn bound_routine() -> StatusView {
    StatusView {
        profile: "analytics".into(),
        included: Vec::new(),
        model: "qwen".into(),
        approval_mode: "read-only".into(),
        agent_mode: "build".into(),
        workspace_root: Some("/home/user/proj".into()),
        sharing_on: false,
        host_composed: false,
        denied_programs: Vec::new(),
        task_summary: None,
    }
}

#[test]
fn the_row_always_states_read_only_database_access() {
    for mut view in [bound_routine(), bound_routine()] {
        view.workspace_root = None;
        assert!(
            context_words_for_test(&bound_routine(), 80).contains("Database read-only"),
            "bound session states the posture"
        );
        assert!(
            context_words_for_test(&view, 80).contains("Database read-only"),
            "unbound session states the posture"
        );
    }
}

#[test]
fn an_unbound_workspace_says_so_rather_than_showing_nothing() {
    let mut unbound = bound_routine();
    unbound.workspace_root = None;
    assert!(
        context_words_for_test(&unbound, 80).contains("No workspace bound"),
        "None root says so"
    );
    assert!(
        context_words_for_test(&bound_routine(), 80).contains("Workspace /home/user/proj"),
        "Some root names the binding"
    );
}

#[test]
fn unusual_conditions_appear_as_words_not_colour() {
    let mut view = bound_routine();
    view.sharing_on = true;
    view.approval_mode = "bypass".into();
    view.host_composed = true;
    view.denied_programs = vec!["curl".into(), "wget".into()];
    let words = context_words_for_test(&view, 160);
    for phrase in [
        "Data sharing on",
        "Approval: bypass",
        "Host commands unsandboxed",
        "Denied: curl, wget",
    ] {
        assert!(words.contains(phrase), "row states {phrase:?} in words");
    }
    // Ignoring every Style leaves the content unchanged: the words are
    // present with or without colour, so colour is decoration only.
    let unstyled: String = context_spans_for_test(&view, 160)
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert_eq!(unstyled, words, "styles carry no content");
}

#[test]
fn a_routine_session_carries_no_extra_segments() {
    let words = context_words_for_test(&bound_routine(), 80);
    for phrase in [
        "Data sharing on",
        "Approval:",
        "Host commands unsandboxed",
        "Denied:",
    ] {
        assert!(!words.contains(phrase), "routine row omits {phrase:?}");
    }
}

#[test]
fn a_narrow_row_keeps_the_posture_over_optional_chrome() {
    let mut view = bound_routine();
    view.sharing_on = true;
    view.approval_mode = "bypass".into();
    view.host_composed = true;
    view.denied_programs = vec!["curl".into()];
    let words = context_words_for_test(&view, 20);
    assert!(
        words.contains("Database read-only"),
        "width 20 keeps the posture legible"
    );
    assert!(
        !words.contains("saya") && !words.contains("Workspace"),
        "width 20 drops the lead and the binding, not the posture: {words}"
    );
}

/// A narrow terminal must not be the reason a caveat stops being visible.
/// At width 40 there is room for the posture and one warning, but not for
/// the `saya` lead and a long workspace path as well — so those go first.
/// The earlier ranking dropped all four warnings and kept the app's own
/// name, which is exactly backwards for the row that exists to say what
/// the session can touch.
#[test]
fn a_warning_outranks_the_lead_and_the_workspace_path() {
    let mut view = bound_routine();
    view.sharing_on = true;
    let words = context_words_for_test(&view, 40);
    assert!(
        words.contains("Data sharing on"),
        "a warning survives a narrow row while chrome is dropped: {words}"
    );
    assert!(
        words.contains("Database read-only"),
        "the posture survives alongside it: {words}"
    );
    assert!(
        !words.contains("saya"),
        "the lead is dropped first — it is pure chrome: {words}"
    );
}

/// Within the warnings themselves the last stated goes first, so the
/// ordering is stable and `Data sharing on` outlives `Denied:`.
#[test]
fn warnings_are_dropped_last_stated_first() {
    let mut view = bound_routine();
    view.sharing_on = true;
    view.denied_programs = vec!["curl".into()];
    let words = context_words_for_test(&view, 45);
    assert!(
        words.contains("Data sharing on") && !words.contains("Denied"),
        "the later warning is shed before the earlier one: {words}"
    );
}

/// The approval segment keeps the approval mode's own colour, so `bypass`
/// ("every call runs without asking") and `never` read as danger at a
/// glance, not as the same amber as `ask`. The bottom bar no longer names
/// the mode, so this row is the only place the signal can live.
#[test]
fn the_approval_segment_is_coloured_by_its_mode() {
    use super::super::status::approval_colour;
    for mode in ["ask", "never", "bypass"] {
        let mut view = bound_routine();
        view.approval_mode = mode.into();
        let spans = context_spans_for_test(&view, 200);
        let segment = spans
            .iter()
            .find(|span| span.content.starts_with("Approval:"))
            .unwrap_or_else(|| panic!("the row names a non-read-only approval ({mode})"));
        assert_eq!(
            segment.style.fg,
            Some(approval_colour(mode)),
            "Approval: {mode} must carry its mode's colour"
        );
    }
}
