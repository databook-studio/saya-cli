/// Between the key prompt and the next prompt the flow prints exactly the
/// presence sentence — no echoed value, nothing else.
#[test]
fn the_presence_line_never_prints_a_value() {
    let dir = temp_dir("key-presence");
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\nSAYA_TEST_UNSET_KEY_7F3A91\n"),
    );
    assert_eq!(code, 0, "{out}");
    let prompt = "API key environment variable name (blank to skip; never the key itself): ";
    let start = out.find(prompt).expect("the key prompt is shown") + prompt.len();
    let end = out
        .find("Choose a database:")
        .expect("the database menu follows the provider");
    assert_eq!(
        &out[start..end],
        "SAYA_TEST_UNSET_KEY_7F3A91 is not set.\n",
        "only the name and the presence sentence sit between the prompts"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A malformed name is refused once and re-asked; three invalid answers burn
/// the bounded retry and cancel with nothing written.
#[test]
fn a_malformed_key_name_is_refused_and_reasked() {
    let dir = temp_dir("key-malformed");
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\nmy-key\nSAYA_TEST_UNSET_KEY_7F3A91\n"),
    );
    assert_eq!(code, 0, "{out}");
    let reason = "api key environment variable name \"my-key\" must match [A-Z_][A-Z0-9_]{0,63}";
    assert_eq!(
        out.matches(reason).count(),
        1,
        "the refusal is said once, then the prompt is asked again: {out}"
    );
    assert!(
        out.contains("SAYA_TEST_UNSET_KEY_7F3A91 is not set."),
        "the flow continues after the refusal: {out}"
    );
    let config = written_config(&dir, &out);
    assert!(
        config.contains("api_key = { env = \"SAYA_TEST_UNSET_KEY_7F3A91\" }"),
        "{config}"
    );
    let _ = fs::remove_dir_all(dir);

    let dir = temp_dir("key-malformed-cancel");
    let (code, out) = key_flow(
        &dir,
        "3\n\nhttps://compat.example.com/v1\nmy-key\nmy-key\nmy-key\n",
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("No files were changed."), "{out}");
    assert!(
        !dir.join("config.toml").exists(),
        "a cancelled flow writes nothing"
    );
    let _ = fs::remove_dir_all(dir);
}

/// Ollama is local: no key question at all, and no key in the written section.
#[test]
fn ollama_never_asks_for_a_key() {
    let dir = temp_dir("key-ollama");
    let (code, out) = key_flow(&dir, &provider_tail("1\n\n\n"));
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("API key environment variable name"), "{out}");
    let config = written_config(&dir, &out);
    assert!(config.contains("provider = \"ollama\""), "{config}");
    assert!(
        config.contains("base_url = \"http://localhost:11434\""),
        "{config}"
    );
    assert!(!config.contains("api_key"), "{config}");
    let _ = fs::remove_dir_all(dir);
}

/// Whatever the wizard collects, an existing config.toml keeps its bytes: the
/// new endpoint's section goes into the note only, without the old key.
#[test]
fn an_existing_config_is_never_rewritten_whatever_provider_is_chosen() {
    let existing =
        b"[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o-mini\"\napi_key = { env = \"OPENAI_API_KEY\" }\n"
            .to_vec();

    let dir = temp_dir("config-kept-blank");
    fs::write(dir.join("config.toml"), &existing).unwrap();
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\n\n"),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read(dir.join("config.toml")).unwrap(),
        existing,
        "the existing config bytes are never rewritten"
    );
    assert!(
        out.contains("note:"),
        "the section is offered as a note: {out}"
    );
    assert!(
        out.contains("base_url = \"https://compat.example.com/v1\""),
        "{out}"
    );
    assert!(
        !out.contains("api_key"),
        "the old key reference is not carried into the new endpoint's snippet: {out}"
    );
    assert!(!out.contains("OPENAI_API_KEY"), "{out}");
    let _ = fs::remove_dir_all(dir);

    let dir = temp_dir("config-kept-named");
    fs::write(dir.join("config.toml"), &existing).unwrap();
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\nSAYA_TEST_UNSET_KEY_7F3A91\n"),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(fs::read(dir.join("config.toml")).unwrap(), existing);
    assert!(out.contains("note:"), "{out}");
    assert!(
        out.contains("api_key = { env = \"SAYA_TEST_UNSET_KEY_7F3A91\" }"),
        "the note carries the name the user typed: {out}"
    );
    assert!(!out.contains("OPENAI_API_KEY"), "{out}");
    let _ = fs::remove_dir_all(dir);
}

/// The review shows the exact `api_key` reference before the write is
/// confirmed, so the user sees what will land in config.toml.
#[test]
fn the_review_shows_the_exact_reference_it_will_write() {
    let dir = temp_dir("key-review");
    let (code, out) = key_flow(&dir, &provider_tail("2\n\n\n"));
    assert_eq!(code, 0, "{out}");
    let confirmed = out
        .find("Write these files?")
        .expect("the confirm is asked");
    let review = &out[..confirmed];
    assert!(
        review.contains("api_key = { env = \"OPENAI_API_KEY\" }"),
        "the user sees the exact reference before writing: {review}"
    );
    assert!(review.contains("provider = \"openai\""), "{review}");
    let _ = fs::remove_dir_all(dir);
}
