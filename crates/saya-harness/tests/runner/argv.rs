use super::*;

/// Every corpus element arrives byte-exact and as exactly one argv
/// element. The assertion is on what the child received — decoded from
/// its own dump — never on the absence of a crash.
#[tokio::test]
async fn the_argv_corpus_passes_verbatim_one_element_each() {
    let corpus = [
        "semi;colon".to_owned(),
        "dollar$(id)".to_owned(),
        "back`tick`".to_owned(),
        "two words".to_owned(),
        "line1\nline2".to_owned(),
    ];
    let mut call = vec!["argv".to_owned()];
    call.extend(corpus.iter().cloned());
    let outcome = standard()
        .tool()
        .run(PROGRAM, &call, Some(30))
        .await
        .expect("the argv dumper must run");
    let received = received_argv(&outcome);
    // received[0] is the program path (argv[0]), received[1] the mode.
    assert_eq!(received[1], "argv");
    for (index, element) in corpus.iter().enumerate() {
        assert_eq!(
            &received[2 + index],
            element,
            "corpus element {index} must arrive as exactly one argv element, byte for byte"
        );
    }
}
