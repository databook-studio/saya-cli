/// The CLI grammar parses and the flags land on the `Open` variant. This test
/// also compiles against `Command::Open`, so the variant's shape is pinned.
#[test]
fn open_grammar_parses_and_pins_the_flags() {
    let parsed = Cli::try_parse_from([
        "saya",
        "open",
        "data/sales.csv",
        "--delimiter",
        ";",
        "--no-header",
        "--typed",
    ])
    .expect("open parses its grammar");
    match parsed.command {
        Some(saya_cli::Command::Open {
            file,
            delimiter,
            no_header,
            reset,
            typed,
            list,
            cleanup,
        }) => {
            assert_eq!(file.as_deref(), Some(Path::new("data/sales.csv")));
            assert_eq!(delimiter.as_deref(), Some(";"));
            assert!(no_header);
            assert!(!reset);
            assert!(typed);
            assert!(!list);
            assert!(cleanup.is_none());
        }
        other => panic!("expected Command::Open, got {other:?}"),
    }
    let listed = Cli::try_parse_from(["saya", "open", "--list"]).expect("--list parses");
    assert!(
        matches!(
            listed.command,
            Some(saya_cli::Command::Open { list: true, .. })
        ),
        "--list lands on the Open variant: {listed:?}"
    );
    let cleaned =
        Cli::try_parse_from(["saya", "open", "--cleanup", "abc123"]).expect("--cleanup parses");
    let Some(saya_cli::Command::Open {
        cleanup: Some(prefix),
        ..
    }) = cleaned.command
    else {
        panic!(
            "expected Command::Open with --cleanup, got {:?}",
            cleaned.command
        );
    };
    assert_eq!(prefix, "abc123");
}
