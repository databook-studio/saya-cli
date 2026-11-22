#[cfg(test)]
mod plain_env_tests {
    use super::*;

    /// The environment spelling is one reading of one value: `1`, `true`,
    /// `yes`, `on` — in any case, with the edges trimmed — ask for the plain
    /// REPL, and nothing else does. The table is the whole grammar, so a
    /// changed spelling fails here rather than silently in a launch.
    #[test]
    fn only_the_yes_spellings_ask_for_the_plain_repl() {
        for value in ["1", "true", "TRUE", " yes ", "On", "yes\n"] {
            assert!(
                says_plain(Some(value.into())),
                "{value:?} asks for the plain REPL"
            );
        }
        for value in ["", "0", "false", "no", "off", "pls", "2"] {
            assert!(
                !says_plain(Some(value.into())),
                "{value:?} states nothing"
            );
        }
        assert!(!says_plain(None), "an unset variable states nothing");
    }

    /// The flag alone states the request: `--plain` asks for the line REPL
    /// without any variable set (the flag short-circuits, so this assertion
    /// does not depend on the ambient environment). The variable's own
    /// spellings are the table above.
    #[test]
    fn the_flag_alone_states_the_request() {
        let flagged = GlobalOptions {
            plain: true,
            ..Default::default()
        };
        assert!(
            plain_requested(&flagged),
            "`--plain` asks for the line REPL without any variable set"
        );

        let bare = GlobalOptions::default();
        assert!(
            !bare.plain,
            "an unstated flag leaves the launch on the TUI"
        );
    }
}
