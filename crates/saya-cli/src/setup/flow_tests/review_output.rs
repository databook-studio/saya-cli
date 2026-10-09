// ---------------------------------------------------------------- review ----

#[test]
fn review_prints_notes_and_created_files() {
    let dir = temp_dir("review");
    fs::write(dir.join("config.toml"), "[ai]\nprovider = \"ollama\"\n").unwrap();
    let draft = SetupDraft {
        provider: Some(ProviderDraft {
            provider: AiProvider::Openai,
            model: "gpt-4o-mini".into(),
            base_url: None,
            api_key_env: Some("OPENAI_API_KEY".into()),
        }),
        profile: None,
    };
    let planned = plan(&dir, &draft).unwrap();
    assert!(planned.writes.is_empty(), "config.toml exists: note only");
    let text = review::render(&dir, &planned);
    assert!(text.contains("note:"), "the note is printed: {text}");
    assert!(
        text.contains("api_key = { env = \"OPENAI_API_KEY\" }"),
        "the note carries the exact snippet: {text}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn review_prints_the_full_text_for_a_new_file() {
    let dir = temp_dir("review-create");
    let draft = SetupDraft {
        provider: None,
        profile: Some(ProfileDraft {
            name: "team".into(),
            profile: DatabaseProfile::Sqlite {
                path: "/tmp/t.db".into(),
                read_only: true,
            },
        }),
    };
    let planned = plan(&dir, &draft).unwrap();
    let text = review::render(&dir, &planned);
    assert!(text.contains("create"), "the action is named: {text}");
    assert!(text.contains("[profiles.team]"), "the block: {text}");
    assert!(
        text.contains("read_only = true"),
        "the exact new text: {text}"
    );
    let _ = fs::remove_dir_all(dir);
}
