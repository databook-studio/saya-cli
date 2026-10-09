//! The argv contract: shaped arguments with metacharacters, spaces,
//! newlines, and quotes arrive as exactly one argv element each, byte for
//! byte.

use super::{config, leak, stage};

use saya_harness::host::HostCommand;

/// Decodes the helper's `argv` mode output back into what the child
/// actually received.
fn received_argv(text: &str) -> Vec<String> {
    let mut received = Vec::new();
    let mut argc = None;
    for line in text.lines() {
        if let Some(count) = line.strip_prefix("argc=") {
            argc = Some(count.parse::<usize>().expect("argc is a number"));
        } else if let Some((index, hex)) = line
            .strip_prefix("arg")
            .and_then(|rest| rest.split_once('='))
        {
            assert_eq!(
                index.parse::<usize>().expect("arg index"),
                received.len(),
                "argv elements arrive in order"
            );
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex byte"))
                .collect::<Vec<u8>>();
            received.push(String::from_utf8(bytes).expect("the corpus is UTF-8"));
        }
    }
    assert_eq!(
        argc,
        Some(received.len()),
        "argc must match the decoded elements: {received:?}"
    );
    received
}

#[tokio::test]
async fn typed_argv_arrives_byte_exact() {
    let dir = leak("argv");
    stage(&dir, "argv-probe");
    let config = config(&dir.display().to_string(), &[]);
    let corpus = [
        "semi;colon".to_owned(),
        "dollar$(id)".to_owned(),
        "back`tick`".to_owned(),
        "two words".to_owned(),
        "line1\nline2".to_owned(),
        "quote\"single'".to_owned(),
        "glob*.txt".to_owned(),
        "dash--flag".to_owned(),
    ];
    let mut argv = vec!["argv".to_owned()];
    argv.extend(corpus.iter().cloned());
    let outcome = HostCommand::new("argv-probe", argv)
        .expect("shaped")
        .run(&config, None, &saya_agent::CancellationToken::new())
        .await
        .expect("the argv dumper must run");
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "stderr {:?}",
        outcome.stderr.text
    );
    let received = received_argv(&outcome.stdout.text);
    assert_eq!(received[1], "argv");
    for (index, element) in corpus.iter().enumerate() {
        assert_eq!(
            &received[2 + index],
            element,
            "corpus element {index} must arrive as exactly one argv element, byte for byte"
        );
    }
}
