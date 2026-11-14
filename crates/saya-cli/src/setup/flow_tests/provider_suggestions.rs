/// A cloud provider suggests exactly its own conventional key NAME; a blank
/// answer accepts it; the written reference is a name, never a value.
#[test]
fn openai_suggests_its_own_key_name_and_a_blank_answer_accepts_it() {
    let dir = temp_dir("key-openai");
    let (code, out) = key_flow(&dir, &provider_tail("2\n\n\n"));
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("API key environment variable name [OPENAI_API_KEY] (never the key itself): "),
        "{out}"
    );
    assert!(
        out.contains("OPENAI_API_KEY is not set.")
            || out.contains("OPENAI_API_KEY is currently set."),
        "the presence line names the variable only: {out}"
    );
    let config = written_config(&dir, &out);
    assert!(config.contains("provider = \"openai\""), "{config}");
    assert!(
        config.contains("api_key = { env = \"OPENAI_API_KEY\" }"),
        "{config}"
    );
    assert!(
        !config.contains("base_url"),
        "openai carries no base URL: {config}"
    );
    assert!(
        !config.contains("api_key = \""),
        "never an inline value: {config}"
    );
    assert!(!out.contains("sk-"), "no secret-looking value shown: {out}");
    let _ = fs::remove_dir_all(dir);
}

/// Anthropic and Gemini each suggest only their own name, never another
/// provider's.
#[test]
fn anthropic_and_gemini_suggest_their_own_names() {
    for (choice, provider, name, others) in [
        (
            "4",
            "anthropic",
            "ANTHROPIC_API_KEY",
            ["OPENAI_API_KEY", "GEMINI_API_KEY"],
        ),
        (
            "5",
            "gemini",
            "GEMINI_API_KEY",
            ["OPENAI_API_KEY", "ANTHROPIC_API_KEY"],
        ),
    ] {
        let dir = temp_dir(&format!("key-{provider}"));
        let (code, out) = key_flow(&dir, &provider_tail(&format!("{choice}\n\n\n")));
        assert_eq!(code, 0, "{provider}: {out}");
        assert!(
            out.contains(&format!(
                "API key environment variable name [{name}] (never the key itself): "
            )),
            "{provider}: {out}"
        );
        for other in others {
            assert!(
                !out.contains(other),
                "{provider} never shows {other}: {out}"
            );
        }
        let config = written_config(&dir, &out);
        assert!(
            config.contains(&format!("provider = \"{provider}\"")),
            "{config}"
        );
        assert!(
            config.contains(&format!("api_key = {{ env = \"{name}\" }}")),
            "{config}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

/// A compatible endpoint gets no suggested name and a blank answer skips the
/// key entirely: no reference, no presence line.
#[test]
fn a_compatible_endpoint_gets_no_suggested_key_and_a_blank_answer_skips_it() {
    let dir = temp_dir("key-compat-blank");
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\n\n"),
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("API key environment variable name (blank to skip; never the key itself): "),
        "{out}"
    );
    assert!(
        !out.contains("API key environment variable name ["),
        "no bracketed suggestion for a compatible endpoint: {out}"
    );
    assert!(
        !out.contains(" is not set.") && !out.contains(" is currently set."),
        "a skipped key prints no presence line: {out}"
    );
    assert!(!out.contains("OPENAI_API_KEY"), "{out}");
    let config = written_config(&dir, &out);
    assert!(
        config.contains("base_url = \"https://compat.example.com/v1\""),
        "{config}"
    );
    assert!(
        !config.contains("api_key"),
        "a blank answer skips the key: {config}"
    );
    assert!(!config.contains("OPENAI_API_KEY"), "{config}");
    let _ = fs::remove_dir_all(dir);
}

/// A compatible endpoint writes exactly the env NAME the user typed.
#[test]
fn a_compatible_endpoint_uses_only_the_name_the_user_types() {
    let dir = temp_dir("key-compat-named");
    let (code, out) = key_flow(
        &dir,
        &provider_tail("3\n\nhttps://compat.example.com/v1\nSAYA_TEST_UNSET_KEY_7F3A91\n"),
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("SAYA_TEST_UNSET_KEY_7F3A91 is not set."),
        "the presence line reports the name only: {out}"
    );
    let config = written_config(&dir, &out);
    assert!(
        config.contains("api_key = { env = \"SAYA_TEST_UNSET_KEY_7F3A91\" }"),
        "{config}"
    );
    assert!(
        config.contains("base_url = \"https://compat.example.com/v1\""),
        "{config}"
    );
    assert!(!config.contains("OPENAI_API_KEY"), "{config}");
    let _ = fs::remove_dir_all(dir);
}
