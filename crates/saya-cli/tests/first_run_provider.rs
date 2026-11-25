//! First run with only a provider key in the environment: the provider is
//! chosen from that key, its default model is used, and a first answer arrives
//! with no `config.toml`. Two keys are never guessed between, and a provider a
//! layer states wins.

#[allow(dead_code)]
#[path = "first_run_provider/harness.rs"]
mod harness;

use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use harness::{ScriptStep, fixture, run_ask, run_config_show, spawn_script_server};

const DEADLINE: Duration = Duration::from_secs(30);

/// The headline row: install, export a key, run saya. No config file exists
/// when the answer arrives, and the one line says what was chosen and how to
/// change it.
#[test]
fn one_key_in_a_fresh_home_answers_without_a_config_file() {
    let fixture = fixture("one-key");
    assert!(
        !fixture.config_home.join("saya/config.toml").exists(),
        "the fixture must start with no config file"
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut server = spawn_script_server(
        listener,
        vec![ScriptStep::Natural],
        Arc::new(AtomicBool::new(false)),
    );
    let run = run_ask(
        &fixture,
        &server.address,
        &[("OPENAI_API_KEY", "sk-test")],
        DEADLINE,
    );
    server.join();

    assert!(
        !run.killed,
        "the ask must finish; stderr={}\nstdout={}",
        run.stderr, run.stdout
    );
    assert_eq!(
        run.code,
        Some(0),
        "stderr={}\nstdout={}",
        run.stderr,
        run.stdout
    );
    assert!(
        run.stdout.contains("the-final-answer"),
        "the first answer arrives: {}",
        run.stdout
    );
    let announcement = run
        .stderr
        .lines()
        .find(|line| line.contains("OPENAI_API_KEY"))
        .unwrap_or_else(|| panic!("the auto choice is announced: {}", run.stderr));
    assert!(announcement.contains("openai"), "{announcement}");
    assert!(announcement.contains("gpt-4o-mini"), "{announcement}");
    assert!(
        announcement.contains("--provider") || announcement.contains("[ai]"),
        "the line says how to change it: {announcement}"
    );
    assert!(
        !fixture.config_home.join("saya/config.toml").exists(),
        "no config file was written"
    );

    let bodies = server.bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1, "one answering request");
    assert!(
        bodies[0].contains("gpt-4o-mini"),
        "the default model is asked for: {}",
        bodies[0]
    );
    assert!(
        !bodies[0].contains("bounded_sql_query"),
        "data sharing stays off, so database tools are not advertised: {}",
        bodies[0]
    );
}

/// Two keys are a question, never a guess. With no terminal to ask on, the
/// child refuses and names both keys and both ways to choose.
#[test]
fn two_keys_are_refused_and_both_are_named() {
    let fixture = fixture("two-keys");
    let run = run_config_show(
        &fixture,
        &[
            ("OPENAI_API_KEY", "sk-openai"),
            ("ANTHROPIC_API_KEY", "sk-anthropic"),
        ],
        Duration::from_secs(30),
    );
    assert!(
        !run.killed,
        "the refusal must be immediate; stderr={}",
        run.stderr
    );
    assert_ne!(
        run.code,
        Some(0),
        "two keys are not a choice: stdout={}\nstderr={}",
        run.stdout,
        run.stderr
    );
    assert!(run.stderr.contains("OPENAI_API_KEY"), "{}", run.stderr);
    assert!(run.stderr.contains("ANTHROPIC_API_KEY"), "{}", run.stderr);
    assert!(
        run.stderr.contains("--provider") || run.stderr.contains("SAYA_PROVIDER"),
        "the refusal says how to choose: {}",
        run.stderr
    );
}

/// A configured provider always wins: the key is not consulted when the user
/// config already names one.
#[test]
fn a_configured_provider_wins_over_the_key() {
    let fixture = fixture("configured");
    harness::write_config(&fixture, "[ai]\nprovider = 'anthropic'\n");
    let run = run_config_show(
        &fixture,
        &[("OPENAI_API_KEY", "sk-openai")],
        Duration::from_secs(30),
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(
        run.stdout.contains("\"provider\":\"anthropic\""),
        "the configured provider is in force: {}",
        run.stdout
    );
    assert!(
        !run.stderr.contains("Using openai"),
        "no auto choice is announced: {}",
        run.stderr
    );
}

/// `SAYA_PROVIDER` is a statement too, and it outranks a key in the
/// environment exactly as a config file does.
#[test]
fn saya_provider_wins_over_the_key() {
    let fixture = fixture("saya-provider");
    let run = run_config_show(
        &fixture,
        &[
            ("SAYA_PROVIDER", "anthropic"),
            ("OPENAI_API_KEY", "sk-openai"),
        ],
        Duration::from_secs(30),
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(
        run.stdout.contains("\"provider\":\"anthropic\""),
        "SAYA_PROVIDER is in force: {}",
        run.stdout
    );
    assert!(
        !run.stderr.contains("Using openai"),
        "no auto choice is announced: {}",
        run.stderr
    );
}
