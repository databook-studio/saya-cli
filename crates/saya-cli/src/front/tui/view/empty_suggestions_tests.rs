//! The welcome's suggestion source: the names it offers and the names it
//! leaves alone.

use super::{SHOWN, from_entries, neutral, top_level};

#[test]
fn a_directory_is_offered_as_what_it_holds() {
    let suggestions = from_entries(&[String::from("src/")]);
    assert_eq!(
        suggestions,
        vec![String::from("explain what src/ contains")]
    );
}

#[test]
fn a_file_is_offered_as_a_request_to_explain_it() {
    let suggestions = from_entries(&[String::from("README.md")]);
    assert_eq!(suggestions, vec![String::from("explain README.md")]);
}

#[test]
fn a_strange_name_is_skipped_not_rewritten() {
    let entries = vec![
        String::from(".hidden"),
        String::from("a/b"),
        String::from("line\nbreak"),
        String::from("Cargo.toml"),
    ];
    assert_eq!(
        from_entries(&entries),
        vec![String::from("explain Cargo.toml")]
    );
}

#[test]
fn the_list_stops_at_three() {
    let entries: Vec<String> = (0..9).map(|index| format!("file{index}")).collect();
    assert_eq!(from_entries(&entries).len(), SHOWN);
}

#[test]
fn the_top_level_names_are_sorted_and_mark_directories() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-ui4-suggestions");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("src builds");
    std::fs::write(root.join("README.md"), b"# demo\n").expect("README writes");
    std::fs::write(root.join(".hidden"), b"x").expect("hidden writes");

    let names = top_level(root.to_str());
    assert_eq!(
        names,
        vec![String::from("README.md"), String::from("src/")],
        "the names are sorted, directories marked, hidden skipped"
    );
    assert!(
        top_level(Some("/nonexistent-saya-ui4")).is_empty(),
        "a folder that cannot be read offers nothing"
    );
    assert!(top_level(None).is_empty(), "no workspace, no names");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_neutral_words_name_no_path() {
    for example in neutral() {
        assert!(
            !example.contains('/') && !example.contains('.'),
            "a neutral example names no path: {example:?}"
        );
    }
}
