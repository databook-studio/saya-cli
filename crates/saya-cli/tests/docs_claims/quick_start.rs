//! The README's Quick Start is the first thing a new user runs, so its
//! commands and setup shapes are executable claims. The section must carry a
//! path from install to a first answer and a first coding job, name the four
//! provider setups the binary supports, and show a SQLite walkthrough that
//! runs anywhere. The feature list must still name what 0.5.0 ships.

const README: &str = include_str!("../../../../README.md");

/// The Quick Start section, from its heading to the next `## ` heading.
fn quick_start() -> &'static str {
    let start = README
        .find("## Quick start")
        .expect("the README has a `## Quick start` section");
    let rest = &README[start + "## Quick start".len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn quick_start_reaches_a_first_answer_and_a_first_coding_job() {
    let section = quick_start();
    for claim in [
        "saya config init",
        "saya ask",
        "saya start",
        "saya job show",
        "saya job review",
        "saya job apply",
        "/mode",
        "Shift+Tab",
    ] {
        assert!(
            section.contains(claim),
            "the Quick start must show `{claim}`:\n{section}"
        );
    }
}

#[test]
fn quick_start_names_the_supported_provider_setups() {
    let section = quick_start();
    for claim in [
        "openai_compatible",
        "openrouter.ai",
        "anthropic",
        "ollama",
        "api_key",
    ] {
        assert!(
            section.contains(claim),
            "the Quick start must show the `{claim}` provider setup:\n{section}"
        );
    }
}

#[test]
fn quick_start_creates_a_sqlite_sample_that_runs_anywhere() {
    let section = quick_start();
    assert!(
        section.contains("sqlite3"),
        "the Quick start must create its sample database with sqlite3:\n{section}"
    );
    assert!(
        section.contains("connections.toml"),
        "the Quick start must show the SQLite connection profile:\n{section}"
    );
    assert!(
        section.contains("default_profile = \"sample\"") && section.contains("--profile sample"),
        "the Quick start must select the sample profile it just added, or the next commands \
         target the starter's PostgreSQL profile:\n{section}"
    );
}

/// The 0.5.0 feature list: each capability the release ships is named in the
/// README's own words, so a stale feature list is caught here.
#[test]
fn readme_feature_list_covers_the_0_5_0_surface() {
    let readme = README.to_lowercase();
    for claim in [
        "saya job",
        "shell",
        "mcp",
        "hooks",
        "web fetch",
        "web search",
        "permission modes",
        "rewind",
        "limits",
        "reasoning",
    ] {
        assert!(
            readme.contains(claim),
            "the README feature list must name `{claim}`"
        );
    }
}
