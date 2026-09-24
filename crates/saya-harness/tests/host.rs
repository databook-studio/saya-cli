//! H0 — the host executor's named properties. Pure addition: nothing in
//! `runner` is touched, and these tests run on every unix CI host (macOS and
//! Linux); the two group-kill tests gate on unix process groups and compile
//! to nothing elsewhere.

#[cfg(unix)]
mod host_battery {
    use std::{
        fs, io, os::unix::fs::PermissionsExt as _, path::PathBuf, sync::OnceLock, time::Duration,
    };

    use saya_harness::host::{HostCommand, HostConfig, HostError};

    /// The battery's one helper, compiled with the toolchain running the
    /// tests: an argv dumper (hex per element), an env dumper, a forking
    /// daemonizer, and an exit-code setter.
    const HELPER_SOURCE: &str = r#"
use std::io::Write;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("");
    match mode {
        "argv" => {
            println!("argc={}", args.len());
            for (index, arg) in args.iter().enumerate() {
                let hex: String = arg.bytes().map(|b| format!("{b:02x}")).collect();
                println!("arg{index}={hex}");
            }
        }
        "env" => {
            let mut vars: Vec<(String, String)> = std::env::vars().collect();
            vars.sort();
            for (name, value) in vars {
                println!("{name}={value}");
            }
        }
        "fork-daemon" => unsafe {
            if fork() == 0 {
                if fork() == 0 {
                    sleep(60);
                    std::process::exit(0);
                }
                std::process::exit(0);
            }
            sleep(60);
        },
        "exit" => {
            let code: i32 = args.get(2).and_then(|c| c.parse().ok()).unwrap_or(0);
            std::process::exit(code);
        }
        _ => {
            eprintln!("unknown mode");
            std::process::exit(2);
        }
    }
    let _ = std::io::stdout().flush();
}

extern "C" {
    fn fork() -> i32;
    fn sleep(seconds: u32);
}
"#;

    fn helper() -> &'static PathBuf {
        static HELPER: OnceLock<PathBuf> = OnceLock::new();
        HELPER.get_or_init(|| {
            let base = std::env::var("CARGO_TARGET_TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
            let dir = base.join(format!("saya-host-helper-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("helper directory must be creatable");
            let source = dir.join("saya-host-helper.rs");
            let binary = dir.join("saya-host-helper");
            fs::write(&source, HELPER_SOURCE).expect("helper source must be written");
            let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
            let status = std::process::Command::new(rustc)
                .arg("--edition=2021")
                .arg("-o")
                .arg(&binary)
                .arg(&source)
                .status()
                .expect("rustc must be runnable — it is the toolchain running this test");
            assert!(
                status.success(),
                "the battery helper must compile: {status}"
            );
            binary
        })
    }

    fn leak(tag: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("saya-host-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("battery dir must be creatable");
        fs::canonicalize(&path).expect("battery dir must canonicalise")
    }

    /// Stages the helper under `dir` as `name` with an exec bit, returning
    /// the dir for PATH construction.
    fn stage(dir: &std::path::Path, name: &str) -> PathBuf {
        let target = dir.join(name);
        fs::copy(helper(), &target).expect("the helper must stage");
        let mut perms = fs::metadata(&target).expect("staged").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&target, perms).expect("exec bit must set");
        dir.to_path_buf()
    }

    fn config(path: &str, extra_env: &[(&str, &str)]) -> HostConfig {
        HostConfig::new(path, std::env::temp_dir(), Duration::from_secs(600))
            .expect("shaped")
            .with_extra_env(
                extra_env
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
            )
            .expect("shaped")
    }

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

    /// `true` when no member of the child's process group remains: signal 0
    /// to the group returns ESRCH exactly when everything died.
    fn group_gone(pgid: u32) -> bool {
        let result = unsafe { libc::killpg(pgid as libc::pid_t, 0) };
        result == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }

    fn wait_group_gone(pgid: u32) -> bool {
        let start = std::time::Instant::now();
        loop {
            if group_gone(pgid) {
                return true;
            }
            if start.elapsed() > Duration::from_secs(10) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
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

    #[tokio::test]
    async fn a_planted_parent_env_var_never_reaches_the_child() {
        unsafe { std::env::set_var("SAYA_HOST_PROBE_PLANTED", "hunter2-planted") };
        let dir = leak("env");
        stage(&dir, "env-probe");
        let config = config(&dir.display().to_string(), &[]);
        let outcome = HostCommand::new("env-probe", ["env".to_owned()])
            .expect("shaped")
            .run(&config, None, &saya_agent::CancellationToken::new())
            .await
            .expect("the env dumper must run");
        assert_eq!(outcome.exit_code, Some(0));
        assert!(
            !outcome.stdout.text.contains("SAYA_HOST_PROBE_PLANTED"),
            "the planted name must be absent from the child's env: {:?}",
            outcome.stdout.text
        );
        assert!(
            !outcome.stdout.text.contains("hunter2-planted"),
            "the planted value must be absent from the child's env"
        );
    }

    #[tokio::test]
    async fn timeout_kills_the_process_group_including_a_daemonizing_grandchild() {
        let dir = leak("daemon");
        stage(&dir, "daemon-probe");
        let config = HostConfig::new(
            dir.display().to_string(),
            std::env::temp_dir(),
            Duration::from_secs(2),
        )
        .expect("shaped");
        let outcome = HostCommand::new("daemon-probe", ["fork-daemon".to_owned()])
            .expect("shaped")
            .run(&config, None, &saya_agent::CancellationToken::new())
            .await
            .expect("the run itself must complete");
        assert!(outcome.killed_by_timeout, "the timeout must have fired");
        assert_eq!(outcome.exit_code, None, "a killed child has no exit code");
        assert!(
            wait_group_gone(outcome.pid),
            "the double-forked grandchild must be gone with the group"
        );
    }

    #[tokio::test]
    async fn resolution_uses_the_built_path_not_the_parent_s() {
        // A program on the parent's PATH (a temp dir prepended to it) but
        // absent from the built PATH must not resolve.
        let dir = leak("parent-path");
        stage(&dir, "only-on-parent-path");
        let old_path = std::env::var("PATH").unwrap_or_default();
        let joined = format!("{}:{}", dir.display(), old_path);
        unsafe { std::env::set_var("PATH", &joined) };
        let empty = leak("built-path-empty");
        let config = config(&empty.display().to_string(), &[]);
        let error = HostCommand::new("only-on-parent-path", Vec::<String>::new())
            .expect("shaped")
            .run(&config, None, &saya_agent::CancellationToken::new())
            .await
            .expect_err("a name off the built PATH must refuse");
        unsafe { std::env::set_var("PATH", &old_path) };
        assert!(
            matches!(&error, HostError::NotOnPath { program, .. } if program == "only-on-parent-path"),
            "the refusal must be the typed not-on-PATH variant: {error}"
        );
    }

    #[tokio::test]
    async fn path_shaped_and_traversal_names_refuse_at_every_layer() {
        let dir = leak("shapes");
        stage(&dir, "probe");
        let searched = dir.display().to_string();
        for name in [
            "./probe",
            "/bin/probe",
            "../probe",
            "sub/dir",
            "probe/",
            ".",
            "..",
        ] {
            let parse = HostCommand::new(name, Vec::<String>::new());
            assert!(
                matches!(&parse, Err(HostError::NameNotBare { .. })),
                "{name} must refuse at construction"
            );
            let config = config(&searched, &[]);
            let error = config
                .resolve(name)
                .expect_err(&format!("{name} must refuse at resolution"));
            assert!(
                matches!(&error, HostError::NameNotBare { .. }),
                "{name} must refuse at resolution: {error}"
            );
        }
    }

    #[tokio::test]
    async fn a_name_not_on_the_built_path_refuses_with_the_path_it_searched() {
        let empty = leak("no-such");
        let config = config(&empty.display().to_string(), &[]);
        let error = HostCommand::new("no-such-program", Vec::<String>::new())
            .expect("shaped")
            .run(&config, None, &saya_agent::CancellationToken::new())
            .await
            .expect_err("a name off the PATH must refuse");
        let HostError::NotOnPath { program, searched } = &error else {
            panic!("the refusal must be the typed not-on-PATH variant: {error}");
        };
        assert_eq!(program, "no-such-program");
        assert!(
            searched.contains(&empty.display().to_string()),
            "the error must name where it looked: {error}"
        );
    }

    #[tokio::test]
    async fn a_wider_timeout_than_the_ceiling_refuses_with_both_numbers() {
        let dir = leak("ceiling");
        stage(&dir, "ceiling-probe");
        let config = config(&dir.display().to_string(), &[]);
        let error = HostCommand::new("ceiling-probe", ["exit".to_owned(), "0".to_owned()])
            .expect("shaped")
            .run(&config, Some(601), &saya_agent::CancellationToken::new())
            .await
            .expect_err("a request above the ceiling is refused");
        assert!(
            matches!(
                &error,
                HostError::TimeoutExceedsCeiling {
                    requested: 601,
                    ceiling: 600
                }
            ),
            "the refusal must carry both numbers: {error}"
        );
        let outcome = HostCommand::new("ceiling-probe", ["exit".to_owned(), "0".to_owned()])
            .expect("shaped")
            .run(&config, Some(30), &saya_agent::CancellationToken::new())
            .await
            .expect("a request at or below the ceiling narrows");
        assert_eq!(outcome.exit_code, Some(0));
    }

    #[tokio::test]
    async fn an_explicitly_passed_var_reaches_the_child() {
        let dir = leak("pass");
        stage(&dir, "pass-probe");
        let config = config(
            &dir.display().to_string(),
            &[("SAYA_HOST_EXTRA", "extra-value")],
        );
        let outcome = HostCommand::new("pass-probe", ["env".to_owned()])
            .expect("shaped")
            .run(&config, None, &saya_agent::CancellationToken::new())
            .await
            .expect("the env dumper must run");
        assert_eq!(outcome.exit_code, Some(0));
        assert!(
            outcome.stdout.text.contains("SAYA_HOST_EXTRA=extra-value"),
            "an explicitly passed var must reach the child: {:?}",
            outcome.stdout.text
        );
    }

    #[tokio::test]
    async fn a_directory_on_the_path_does_not_resolve() {
        let dir = leak("dir-entry");
        fs::create_dir_all(dir.join("a-dir")).expect("a dir entry must plant");
        let config = config(&dir.display().to_string(), &[]);
        let error = HostCommand::new("a-dir", Vec::<String>::new())
            .expect("shaped")
            .run(&config, None, &saya_agent::CancellationToken::new())
            .await
            .expect_err("a directory must refuse, not exec");
        assert!(
            matches!(&error, HostError::NotOnPath { .. }),
            "a directory on the PATH must refuse as not resolvable: {error}"
        );
    }

    #[tokio::test]
    async fn the_child_runs_with_the_bound_root_as_its_cwd() {
        // The prompt's `cwd: pinned to <root>` line is a fact only because
        // the executor applies it: the child must observe the bound root as
        // its own working directory, even when the parent runs elsewhere.
        // The assertion reads the child's `pwd` output and the file it
        // writes — never the prompt string, which states the same line
        // under today's bug.
        let root = leak("cwd-root");
        let elsewhere = leak("cwd-elsewhere");
        let probe = elsewhere.join("probe.sh");
        fs::write(
            &probe,
            "#!/bin/sh\n/bin/pwd\n/usr/bin/touch child-wrote-here\n",
        )
        .expect("the cwd probe must plant");
        let mut perms = fs::metadata(&probe).expect("staged").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&probe, perms).expect("exec bit must set");
        let config = HostConfig::new(
            elsewhere.display().to_string(),
            root.clone(),
            Duration::from_secs(600),
        )
        .expect("shaped");
        let outcome = HostCommand::new("probe.sh", Vec::<String>::new())
            .expect("shaped")
            .run(&config, None, &saya_agent::CancellationToken::new())
            .await
            .expect("the cwd probe must run");
        assert_eq!(
            outcome.exit_code,
            Some(0),
            "stderr {:?}",
            outcome.stderr.text
        );
        let observed = std::path::PathBuf::from(outcome.stdout.text.trim());
        let canonical =
            |path: &std::path::Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        assert_eq!(
            canonical(&observed),
            canonical(&root),
            "the child's own working directory is the bound root"
        );
        assert!(
            root.join("child-wrote-here").exists(),
            "the child's relative write landed in the bound root"
        );
        assert!(
            !elsewhere.join("child-wrote-here").exists(),
            "the child's relative write did not land beside the program"
        );
    }
}
