//! What a message carries: the files its mentions name, bounded per file and in
//! total, refused where the file tools refuse them — and the listing the popup
//! offers, which is the file tools' own.

use std::fs;
use std::path::{Path, PathBuf};

use saya_harness::workspace::Workspace;

use super::{HEADER, MAX_FILE_BYTES, MAX_TOTAL_BYTES, attach, candidates, private_sentence};

/// A workspace under `./target`, removed when it drops: the rule for every
/// temporary file this repository's tests write.
struct Fixture {
    root: PathBuf,
    ws: Workspace,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = Path::new("target").join(format!("tmp-mentions-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).expect("fixture directory must create");
        let ws = Workspace::open(&root).expect("workspace root must open");
        Self { root, ws }
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let at = self.root.join(path);
        fs::create_dir_all(at.parent().expect("a parent")).expect("parent must create");
        fs::write(at, bytes).expect("fixture file must write");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A mentioned file rides the message: the model reads it without a tool call.
#[test]
fn a_mentioned_file_rides_the_message() {
    let fixture = Fixture::new("rides");
    fixture.write("src/main.rs", b"fn main() {}\n");
    let carried = attach("what does @src/main.rs do?", Some(&fixture.ws));
    assert!(
        carried.starts_with("what does @src/main.rs do?"),
        "{carried}"
    );
    assert!(carried.contains(HEADER), "{carried}");
    assert!(
        carried.contains("--- src/main.rs (13 bytes) ---"),
        "{carried}"
    );
    assert!(carried.contains("fn main() {}"), "{carried}");
}

/// A message that mentions nothing is the words that were typed: the request's
/// budget is exactly what it was before mentions existed.
#[test]
fn a_message_without_mentions_carries_nothing() {
    let fixture = Fixture::new("plain");
    fixture.write("src/main.rs", b"fn main() {}\n");
    let message = "explain the composer";
    assert_eq!(attach(message, Some(&fixture.ws)), message);
    // And a run with no workspace carries nothing either, whatever was typed.
    assert_eq!(attach(message, None), message);
}

/// A file over the per-file bound is named with its size, never truncated into
/// the message: the model narrows with a ranged read instead of trusting half
/// a file.
#[test]
fn an_over_bound_file_is_named_with_its_size() {
    let fixture = Fixture::new("over");
    let size = MAX_FILE_BYTES as usize + 1;
    fixture.write("src/big.rs", &vec![b'x'; size]);
    let carried = attach("look at @src/big.rs", Some(&fixture.ws));
    assert!(
        carried.contains(&format!(
            "{size} bytes is over the {MAX_FILE_BYTES}-byte mention bound"
        )),
        "{carried}"
    );
    assert!(
        !carried.contains("xxxx"),
        "an over-bound file's bytes are not carried: {carried}"
    );
}

/// The total bound is spent once across every file: what does not fit is named
/// with the reason, so the model knows what it was not given.
#[test]
fn the_total_bound_is_spent_once_across_the_files() {
    let fixture = Fixture::new("total");
    let each = MAX_FILE_BYTES as usize;
    for index in 0..(MAX_TOTAL_BYTES / each + 1) {
        fixture.write(&format!("src/p{index}.rs"), &vec![b'x'; each]);
    }
    let message: String = (0..(MAX_TOTAL_BYTES / each + 1))
        .map(|index| format!("@src/p{index}.rs "))
        .collect();
    let carried = attach(&message, Some(&fixture.ws));
    assert!(
        carried.contains(&format!(
            "not attached: the {MAX_TOTAL_BYTES}-byte mention total is spent"
        )),
        "{carried}"
    );
}

/// A private path is refused with the read tool's own sentence — the same
/// string a `read` of that path answers with.
#[test]
fn a_private_path_is_refused_with_the_read_tools_sentence() {
    let fixture = Fixture::new("private");
    fixture.write(".env", b"TOKEN=placeholder\n");
    fixture.write("id_rsa", b"placeholder key material\n");
    let carried = attach("read @.env and @id_rsa", Some(&fixture.ws));
    for path in [".env", "id_rsa"] {
        let sentence = private_sentence(path);
        assert!(
            carried.contains(&sentence),
            "the read tool's own refusal for {path} is in the message: {carried}"
        );
        assert!(
            !carried.contains("TOKEN=placeholder") && !carried.contains("placeholder key"),
            "a private file's bytes never reach the message: {carried}"
        );
    }
    // The sentence is the read tool's: its own error text, unaltered.
    assert_eq!(
        private_sentence(".env"),
        saya_agent::ToolError::WorkspacePrivatePath {
            path: ".env".into()
        }
        .to_string()
    );
}

/// The list the popup offers is the file tools' listing: an ignored path and a
/// private path are never offered.
#[test]
fn the_listing_offers_no_ignored_or_private_path() {
    let fixture = Fixture::new("listing");
    fixture.write(".gitignore", b"target/\n");
    fixture.write("src/main.rs", b"fn main() {}\n");
    fixture.write("target/generated.rs", b"fn generated() {}\n");
    fixture.write(".env", b"TOKEN=placeholder\n");
    fixture.write("id_rsa", b"placeholder key material\n");
    let offered = candidates(&fixture.ws);
    assert!(offered.contains(&"src/main.rs".to_owned()), "{offered:?}");
    assert!(
        offered.contains(&".gitignore".to_owned()),
        "an ordinary file is offered: {offered:?}"
    );
    for path in ["target/generated.rs", ".env", "id_rsa"] {
        assert!(
            !offered.contains(&path.to_owned()),
            "{path} is never offered: {offered:?}"
        );
    }
}
