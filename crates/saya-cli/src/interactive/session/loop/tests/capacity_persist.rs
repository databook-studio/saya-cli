#[cfg(test)]
mod capacity_persist_tests {
    use super::*;
    use crate::interactive::session::lifecycle::capacity_test_support::{
        state_with_turns, temp_store_root,
    };
    use saya_store::SessionCapacityStatus;

    /// A save refused at the cap reports the errored outcome — the sentence
    /// rides the Error event — and never dies: the piped REPL keeps reading.
    /// The refusal also records the fact on the run's save-refusal flag, so
    /// a scripted run's end-of-run save can never repeat the sentence.
    #[test]
    fn a_save_refused_at_the_limit_reports_one_errored_outcome_and_the_loop_continues() {
        let root = temp_store_root("loop-limit");
        let store = FsSessionStore::new(&root);
        let mut state = state_with_turns("cap-loop", 500, 11_500);
        assert_eq!(
            store
                .capacity_of(state.redacted())
                .expect("measures")
                .status,
            SessionCapacityStatus::OverCap,
            "the fixture is genuinely over the cap"
        );
        let mut save_refused = false;
        let outcome = persist(&mut state, &store, RenderFormat::Ndjson, &mut save_refused);
        assert!(
            matches!(outcome, Ok(Some(TurnOutcome::Errored))),
            "the refused save says the sentence and lets the loop continue, not die: {outcome:?}"
        );
        assert!(
            save_refused,
            "the refusal records the fact for the end-of-run save"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_healthy_save_returns_no_override_and_leaves_the_flag_unarmed() {
        let root = temp_store_root("loop-ok");
        let store = FsSessionStore::new(&root);
        let mut state = state_with_turns("cap-loop-ok", 2, 64);
        let mut save_refused = false;
        let outcome = persist(&mut state, &store, RenderFormat::Ndjson, &mut save_refused);
        assert!(
            matches!(outcome, Ok(None)),
            "an ordinary save changes no outcome: {outcome:?}"
        );
        assert!(!state.capacity_warned, "a small record arms nothing");
        assert!(!save_refused, "an ordinary save refuses nothing");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_warning_level_save_arms_the_one_time_notice_flag() {
        let root = temp_store_root("loop-warn");
        let store = FsSessionStore::new(&root);
        let mut state = state_with_turns("cap-loop-warn", 300, 11_500);
        let mut save_refused = false;
        let outcome = persist(&mut state, &store, RenderFormat::Ndjson, &mut save_refused);
        assert!(
            matches!(outcome, Ok(None)),
            "a warn-level record still saves: {outcome:?}"
        );
        assert!(
            state.capacity_warned,
            "the near-limit notice armed after the save"
        );
        assert!(!save_refused, "a warn-level save refuses nothing");
        let _ = std::fs::remove_dir_all(root);
    }
}
