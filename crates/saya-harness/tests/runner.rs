//! The M5-4 escape battery: the red tests are the deliverable, not the
//! spawn. Every guarantee the runner's design makes is a named test here:
//!
//! 1. A non-allowlisted program is a typed error and nothing is spawned.
//! 2. The argv corpus: `;`, `$()`, backticks, spaces, and newlines each
//!    arrive as exactly one argv element — asserted on what the child
//!    received, never on the absence of a crash.
//! 3. `bash -c`, `sh -c`, a wrapper script, and an absolute path to an
//!    allowlisted program's lookalike are each refused, each as its own
//!    named test.
//! 4. A timeout kills a daemonizing grandchild: the child forks and stays
//!    alive, the grandchild detaches, and the whole group is gone.
//! 5. ~4 GB-equivalent of stdout stays ring-buffered, capped, and the
//!    truncation is reported.
//! 6. A planted database credential is absent from the child's actual
//!    environment, not from the config that was passed.
//! 7. An endpoint credential is present only under all four conditions;
//!    removing any one makes it disappear.
//! 8. Captured output reaches the model and disk only after `redact()`.
//!
//! The refusal battery (tests 1, 3) is pure validation and runs on every
//! platform CI covers; the spawn-dependent tests need the startup probe to
//! prove the sandbox, which — per the M5-3 decision this slice consumes —
//! only happens where the platform's sandbox is provable (macOS today).

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod spawn_battery {
    use std::{
        fs, io,
        path::PathBuf,
        process::Command,
        sync::{Arc, OnceLock},
        time::{Duration, Instant},
    };

    use saya_agent::ToolExecutor;
    use saya_harness::runner::sandbox::{RunSandbox, RunnerSpawn, SandboxProvision};
    use saya_harness::runner::{
        Credential, OutputRing, ProgramOutcome, RUN_PROGRAM_TOOL, RunProgram, RunnerError,
        StaticCredentialSource, refuse::validate_call,
    };
    use saya_types::RunnerScope;

    /// The allowlisted runner program: the battery's compiled helper.
    const PROGRAM: &str = "saya-probe";
    /// The tool's configured default timeout in the battery.
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

    /// A directory that lives as long as the test process: the shared
    /// battery provisions are built once and reused by every test.
    fn leak(tag: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("saya-runner-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("battery root must be creatable");
        fs::canonicalize(&path).expect("battery root must canonicalise")
    }

    // -- the compiled helper ------------------------------------------------

    /// The battery's one helper program, compiled with the same toolchain
    /// running the tests: an argv dumper (hex per element), an env dumper,
    /// a cwd printer, a stdout flood, a sleeper, a forking daemonizer, and
    /// an exit-code setter. One binary, staged into the program directory.
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
        "cwd" => println!("{}", std::env::current_dir().unwrap().display()),
        "sleep" => {
            let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            std::thread::sleep(Duration::from_secs(seconds));
        }
        "flood" => {
            let bytes: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let chunk = vec![b'x'; 4096];
            let mut out = std::io::stdout();
            let mut left = bytes;
            while left >= chunk.len() {
                out.write_all(&chunk).unwrap();
                left -= chunk.len();
            }
            out.write_all(&chunk[..left]).unwrap();
            println!("\nflooded");
        }
        "secret" => {
            println!("password=hunter2");
            println!("Authorization: Bearer sk-live-999");
            println!("https://user:hunter2@db.example.com/x");
        }
        "fork-daemon" => unsafe {
            if fork() == 0 {
                sleep(60);
                std::process::exit(0);
            }
            sleep(60);
        },
        "fork-orphan" => unsafe {
            if fork() == 0 {
                sleep(60);
                std::process::exit(0);
            }
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
}

#[cfg(unix)]
extern "C" {
    fn fork() -> i32;
    fn sleep(seconds: u32);
}

use std::time::Duration;
"#;

    fn helper() -> &'static PathBuf {
        static HELPER: OnceLock<PathBuf> = OnceLock::new();
        HELPER.get_or_init(|| {
            let dir = std::env::var("CARGO_TARGET_TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
            let source = dir.join("saya-runner-helper.rs");
            let binary = dir.join("saya-runner-helper");
            fs::write(&source, HELPER_SOURCE).expect("helper source must be written");
            let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
            let status = Command::new(rustc)
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

    // -- the shared, proven battery provisions ------------------------------

    /// One proven sandbox, built once: the startup probe ran in full and
    /// proved this host; its `RunnerSpawn` is the only way a tool exists.
    struct Battery {
        workspace: PathBuf,
        programs: PathBuf,
        provision: SandboxProvision,
    }

    impl Battery {
        fn spawn(&self) -> &RunnerSpawn {
            self.provision
                .spawn()
                .expect("the battery provision must be proven")
        }

        fn tool(&self) -> RunProgram {
            self.tool_with_scope(RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"))
        }

        fn tool_with_scope(&self, scope: RunnerScope) -> RunProgram {
            self.tool_with_scope_and_source(
                scope,
                Arc::new(StaticCredentialSource::new(Vec::<(String, String)>::new())),
            )
        }

        fn tool_with_scope_and_source(
            &self,
            scope: RunnerScope,
            resolver: Arc<StaticCredentialSource>,
        ) -> RunProgram {
            RunProgram::new(self.spawn().clone(), scope, DEFAULT_TIMEOUT, resolver)
        }
    }

    static STANDARD: OnceLock<Battery> = OnceLock::new();
    static WITH_FORK: OnceLock<Battery> = OnceLock::new();

    fn standard() -> &'static Battery {
        STANDARD.get_or_init(|| build_battery("standard", false))
    }

    fn with_fork() -> &'static Battery {
        WITH_FORK.get_or_init(|| build_battery("fork", true))
    }

    /// Builds one battery: a workspace root (the only fs root), a program
    /// directory *outside* the roots — a child must not be able to rewrite
    /// its own allowlist — and the staged helper.
    fn build_battery(tag: &str, fork: bool) -> Battery {
        let workspace = leak(tag);
        let programs = leak(&format!("{tag}-programs"));
        fs::copy(helper(), programs.join(PROGRAM)).expect("the helper must stage");
        let policy = RunSandbox::new([workspace.clone()], Vec::<(String, u16)>::new())
            .expect("a real workspace root constructs");
        // `process-fork` is granted only with a measured reason (spike
        // §4.5: a forking child is denied by default with a literal
        // `fork: Operation not permitted`); the reason becomes the
        // generated profile's own comment.
        let policy = match fork {
            true => policy
                .with_process_fork(
                    "the escape battery daemonizer forks a detached child - measured \
                     deny without the allow: bash fork: Operation not permitted - spike 4.5",
                )
                .expect("the reason carries the safe profile text class"),
            false => policy,
        };
        let provision = policy.prepare(&programs).expect("preparation must run");
        assert!(
            provision.report().proves_runner(),
            "the healthy-host probe must prove the battery:\n{}",
            provision.report().render()
        );
        Battery {
            workspace,
            programs,
            provision,
        }
    }

    // -- assertions on what the child left behind ---------------------------

    /// `true` when no member of the child's process group remains: the
    /// group id is the child's pid (`process_group(0)`), so signal 0 to the
    /// group probing membership returns ESRCH exactly when everything died.
    fn group_gone(pgid: u32) -> bool {
        let result = unsafe { libc::killpg(pgid as libc::pid_t, 0) };
        result == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }

    fn wait_group_gone(pgid: u32) -> bool {
        let start = Instant::now();
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

    /// Decodes the helper's `argv` mode output back into the elements the
    /// child actually received.
    fn received_argv(outcome: &ProgramOutcome) -> Vec<String> {
        assert_eq!(
            outcome.exit_code,
            Some(0),
            "the argv dumper must exit 0: stderr {:?}",
            outcome.stderr.text
        );
        let mut received = Vec::new();
        let mut argc = None;
        for line in outcome.stdout.text.lines() {
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
            "argc must match the number of decoded elements: {received:?}"
        );
        received
    }

    // -- test 1: a non-allowlisted program spawns nothing -------------------

    #[tokio::test]
    async fn a_non_allowlisted_program_is_a_typed_error_and_nothing_is_spawned() {
        let outcome = standard()
            .tool()
            .run("printf", &["hello".to_owned()], None)
            .await;
        assert!(
            matches!(&outcome, Err(RunnerError::ProgramNotAllowlisted { program }) if program == "printf"),
            "the refusal must be the typed not-allowlisted variant: {outcome:?}"
        );
    }

    // -- test 2: the argv corpus --------------------------------------------

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

    // -- test 3: the four named shell refusals -------------------------------

    /// `bash` is refused even when a hand-built scope allowlists it: an
    /// interpreter can spawn arbitrary children with arbitrary argv and
    /// would void the typed-argv contract from inside the allowlist.
    #[test]
    fn bash_is_refused_even_when_allowlisted() {
        let scope = RunnerScope::new(vec!["bash".to_owned(), PROGRAM.to_owned()]).expect("shaped");
        let error = validate_call(
            &scope,
            &standard().programs,
            DEFAULT_TIMEOUT,
            None,
            "bash",
            &["-c".to_owned(), "echo pwned".to_owned()],
        )
        .expect_err("bash must be refused");
        assert!(
            matches!(&error, RunnerError::ProgramRefused { program, .. } if program == "bash"),
            "the refusal must name the program: {error}"
        );
    }

    #[test]
    fn sh_is_refused_even_when_allowlisted() {
        let scope = RunnerScope::new(vec!["sh".to_owned(), PROGRAM.to_owned()]).expect("shaped");
        let error = validate_call(
            &scope,
            &standard().programs,
            DEFAULT_TIMEOUT,
            None,
            "sh",
            &["-c".to_owned(), "echo pwned".to_owned()],
        )
        .expect_err("sh must be refused");
        assert!(
            matches!(&error, RunnerError::ProgramRefused { program, .. } if program == "sh"),
            "the refusal must name the program: {error}"
        );
    }

    /// The wrapper trick: a script that execs an interpreter dies on its
    /// shebang — the interpreter it would exec was never allowlisted.
    #[test]
    fn a_wrapper_script_is_refused() {
        let programs = leak("wrapper-programs");
        let wrapper = programs.join("wrapper");
        fs::write(&wrapper, "#!/bin/sh\nexec /bin/echo \"$@\"\n").expect("the wrapper must plant");
        let scope =
            RunnerScope::new(vec!["wrapper".to_owned(), PROGRAM.to_owned()]).expect("shaped");
        let error = validate_call(&scope, &programs, DEFAULT_TIMEOUT, None, "wrapper", &[])
            .expect_err("a wrapper script must be refused");
        assert!(
            matches!(&error, RunnerError::ProgramRefused { program, reason } if program == "wrapper" && reason.contains("script")),
            "the refusal must say why the wrapper died: {error}"
        );
    }

    /// The absolute-path trick: a lookalike binary planted outside the
    /// program directory, named by its absolute path. The allowlist names
    /// programs, never paths — the call is refused before anything spawns.
    #[test]
    fn an_absolute_path_to_a_lookalike_is_refused() {
        let lookalike = leak("lookalike").join("echo-lookalike");
        fs::copy("/bin/echo", &lookalike).expect("the lookalike must stage");
        let error = validate_call(
            &RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"),
            &standard().programs,
            DEFAULT_TIMEOUT,
            None,
            &lookalike.display().to_string(),
            &[],
        )
        .expect_err("a path-shaped program name must be refused");
        assert!(
            matches!(&error, RunnerError::ProgramRefused { reason, .. } if reason.contains("bare name")),
            "the refusal must say a program is a bare name: {error}"
        );
    }

    // -- test 4: the timeout kills a daemonizing grandchild -------------------

    /// The child forks a detached grandchild and stays alive; the timeout
    /// kills the group and the grandchild goes with it. `process-fork` had
    /// to be granted with a measured reason for this child to fork at all —
    /// the battery provision carries it.
    #[tokio::test]
    async fn a_timeout_kills_a_daemonizing_grandchild() {
        let outcome = with_fork()
            .tool()
            .run(PROGRAM, &["fork-daemon".to_owned()], Some(2))
            .await
            .expect("the run itself must complete");
        assert!(
            outcome.killed_by_timeout,
            "the timeout must have fired: {outcome:?}"
        );
        assert_eq!(outcome.exit_code, None, "a killed child has no exit code");
        assert!(
            wait_group_gone(outcome.pid),
            "the daemonizing grandchild must be gone with the group"
        );
    }

    /// The sibling escape: the child exits 0 and leaves the detached
    /// grandchild running. The timeout never fires — so the runner sweeps
    /// the group after the child is reaped, and reports it.
    #[tokio::test]
    async fn a_child_that_exits_leaving_an_orphan_is_swept_and_reported() {
        let outcome = with_fork()
            .tool()
            .run(PROGRAM, &["fork-orphan".to_owned()], None)
            .await
            .expect("the run itself must complete");
        assert_eq!(outcome.exit_code, Some(0), "the parent exited normally");
        assert!(
            outcome.killed_orphans,
            "the orphaned grandchild must be reported as swept: {outcome:?}"
        );
        assert!(
            wait_group_gone(outcome.pid),
            "the detached grandchild must not survive the child"
        );
    }

    // -- test 5: the ring buffer holds four gigabytes-equivalent --------------

    /// 4 GiB of stdout through the ring: memory stays at the cap, the tail
    /// is the last thing the child wrote, and the truncation is reported —
    /// then a real child flooding 1 MiB through the sandboxed runner is
    /// capped and reported the same way.
    #[test]
    fn four_gigabytes_of_stdout_stay_ring_buffered_capped_and_reported() {
        let cap = 64 * 1024;
        let ring = OutputRing::new(cap);
        let total: u64 = 4 * 1024 * 1024 * 1024;
        let chunk = vec![b'x'; 1024 * 1024];
        let mut written: u64 = 0;
        while written + chunk.len() as u64 <= total {
            ring.push(&chunk);
            written += chunk.len() as u64;
        }
        // The sentinel rides the final bytes: only the ring's tail may
        // remember it.
        let sentinel: &[u8] = b"SAYA-TAIL-SENTINEL";
        let mut last = vec![b'y'; 1024 * 1024];
        let tail_len = last.len() - sentinel.len();
        last[tail_len..].copy_from_slice(sentinel);
        ring.push(&last);
        let (text, truncated, dropped, counted) = ring.snapshot();
        assert_eq!(counted, total + last.len() as u64, "every byte was counted");
        assert_eq!(text.len(), cap, "the ring holds the cap and nothing more");
        assert!(truncated, "the truncation must be reported");
        assert_eq!(
            dropped,
            total + last.len() as u64 - cap as u64,
            "the drop is accounted"
        );
        assert!(
            text.ends_with(std::str::from_utf8(sentinel).unwrap()),
            "the ring keeps the tail: the last bytes fed must be the last bytes kept"
        );
    }

    #[tokio::test]
    async fn a_real_flood_is_capped_and_truncation_is_reported() {
        let outcome = standard()
            .tool()
            .run(
                PROGRAM,
                &["flood".to_owned(), "1048576".to_owned()],
                Some(30),
            )
            .await
            .expect("the flood must complete");
        assert_eq!(outcome.exit_code, Some(0));
        assert!(
            outcome.stdout.truncated,
            "1 MiB through a 64 KiB cap must be reported truncated: {outcome:?}"
        );
        assert!(
            outcome.stdout.dropped_bytes >= 1_048_576 - 64 * 1024,
            "the drop is accounted: {outcome:?}"
        );
        assert!(
            outcome.stdout.text.len() <= 64 * 1024,
            "the retained tail is bounded by the cap: {}",
            outcome.stdout.text.len()
        );
        assert!(
            outcome.stdout.text.trim_end().ends_with("flooded"),
            "the ring keeps the tail — the flood's last line survives: {:?}",
            outcome.stdout.text
        );
    }

    // -- test 6: a planted credential is absent from the child's env ----------

    /// The parent's environment carries a planted database credential; the
    /// child's ACTUAL environment — what it prints, not what was passed —
    /// must not contain it.
    #[tokio::test]
    async fn a_planted_credential_is_absent_from_the_child_environment() {
        // One test plants into the process env; edition 2024 marks the
        // setter unsafe because it is a process-wide mutation.
        unsafe { std::env::set_var("SAYA_PROBE_DB_PASSWORD", "hunter2-planted") };
        let outcome = standard()
            .tool()
            .run(PROGRAM, &["env".to_owned()], Some(30))
            .await
            .expect("the env dumper must run");
        assert_eq!(outcome.exit_code, Some(0), "the child must have run");
        assert!(
            !outcome.stdout.text.contains("SAYA_PROBE_DB_PASSWORD"),
            "the planted credential's name must be absent from the child's actual env: {:?}",
            outcome.stdout.text
        );
        assert!(
            !outcome.stdout.text.contains("hunter2-planted"),
            "the planted credential's value must be absent from the child's actual env"
        );
    }

    // -- test 7: the credential's four conditions ------------------------------

    fn probe_credential() -> Credential {
        use saya_types::SecretRef;
        Credential::new(
            "probe-endpoint",
            "SAYA_PROBE_ENDPOINT_API_KEY",
            SecretRef::Env {
                env: "SAYA_PROBE_SECRET_SOURCE".to_owned(),
            },
        )
        .expect("the declared credential is well-shaped")
    }

    fn credentialed_tool() -> RunProgram {
        let source = StaticCredentialSource::new([(
            "SAYA_PROBE_SECRET_SOURCE".to_owned(),
            "sk-probe-value-123".to_owned(),
        )]);
        standard()
            .tool_with_scope_and_source(
                RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"),
                Arc::new(source),
            )
            .with_credentials(vec![probe_credential()])
    }

    /// Declared, sandboxed, referenced, redacted: the variable is in the
    /// child's actual environment, and the value is scrubbed from the
    /// captured output the model reads (the `api_key=` marker redacts).
    #[tokio::test]
    async fn the_declared_credential_reaches_the_sandboxed_child() {
        let outcome = credentialed_tool()
            .run(PROGRAM, &["env".to_owned()], Some(30))
            .await
            .expect("the run must complete");
        assert_eq!(outcome.exit_code, Some(0));
        assert!(
            outcome.stdout.text.contains("SAYA_PROBE_ENDPOINT_API_KEY="),
            "the declared credential must reach the child's actual env: {:?}",
            outcome.stdout.text
        );
        assert!(
            !outcome.stdout.text.contains("sk-probe-value-123"),
            "the value itself must be scrubbed from captured output: {:?}",
            outcome.stdout.text
        );
    }

    /// Remove "declared in the approved plan": an undeclared credential is
    /// never on the tool's list, so the variable vanishes.
    #[tokio::test]
    async fn an_undeclared_credential_never_reaches_the_child() {
        let outcome = standard()
            .tool_with_scope(RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"))
            .with_credentials(Vec::new())
            .run(PROGRAM, &["env".to_owned()], Some(30))
            .await
            .expect("the run must complete");
        assert!(
            !outcome.stdout.text.contains("SAYA_PROBE_ENDPOINT_API_KEY"),
            "an undeclared credential must be absent from the child's env: {:?}",
            outcome.stdout.text
        );
    }

    /// Remove "sandboxed": where the probe did not prove the sandbox, no
    /// spawn configuration exists and the tool cannot exist at all — the
    /// credential has no path to a child.
    #[test]
    fn an_unsandboxed_host_has_no_runner_to_inject_through() {
        let workspace = leak("unproven-ws");
        let programs = leak("unproven-programs");
        let policy = RunSandbox::new([workspace.clone()], Vec::<(String, u16)>::new())
            .expect("the policy constructs while the root exists");
        fs::remove_dir_all(&workspace).expect("the root vanishes before prepare");
        let provision = policy.prepare(&programs).expect("preparation runs");
        assert!(provision.spawn().is_none(), "no proven sandbox, no spawn");
        assert!(
            !provision.report().failed_required().is_empty(),
            "the report must name what failed"
        );
    }

    /// Remove "references-only": a reference that cannot be resolved fails
    /// the call before anything spawns — a declared credential that
    /// silently did not inject would leave the child to fail in its place.
    #[tokio::test]
    async fn an_unresolvable_reference_injects_nothing_and_spawns_nothing() {
        let error = standard()
            .tool_with_scope(RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"))
            .with_credentials(vec![probe_credential()])
            .run(PROGRAM, &["env".to_owned()], Some(30))
            .await
            .expect_err("the unresolvable reference must fail the call");
        assert!(
            matches!(&error, RunnerError::CredentialUnresolved { credential, .. } if credential == "probe-endpoint"),
            "the failure must name the credential: {error}"
        );
    }

    // -- test 8: redact() gates the model copy and the disk copy ---------------

    /// The child prints credentials on stdout; both places the capture
    /// reaches — the tool's JSON result (the model lane) and the record file
    /// it persists into the run workspace (the disk lane) — are scrubbed.
    #[tokio::test]
    async fn captured_output_is_redacted_in_the_model_result_and_on_disk() {
        let result = credentialed_tool()
            .execute(
                RUN_PROGRAM_TOOL,
                serde_json::json!({
                    "program": PROGRAM,
                    "args": ["secret"],
                    "timeout_seconds": 30
                }),
            )
            .await
            .expect("the call runs");
        let model_text = result["stdout"]["text"]
            .as_str()
            .expect("the model lane's stdout text");
        let record_path = result["record_path"].as_str().expect("the record path");
        let disk_text = fs::read_to_string(record_path).expect("the disk lane must be readable");
        for (lane, text) in [
            ("the model lane", model_text),
            ("the disk lane", disk_text.as_str()),
        ] {
            assert!(
                !text.contains("hunter2"),
                "{lane} must not carry the planted marker secret: {text}"
            );
            assert!(
                !text.contains("sk-live-999"),
                "{lane} must not carry the planted bearer token: {text}"
            );
            assert!(
                text.contains("[redacted]"),
                "{lane} must show the redaction happened: {text}"
            );
        }
    }

    // -- the rest of the shape: narrowing, cwd, exit codes, cancellation -------

    /// The allowlist is per step: a program inside the run's approval but
    /// outside the step's narrowed scope is refused, while the step's own
    /// program runs.
    #[tokio::test]
    async fn the_allowlist_is_the_step_scope_narrowed_from_the_run_approval() {
        let run_approved =
            RunnerScope::new(vec![PROGRAM.to_owned(), "second".to_owned()]).expect("shaped");
        let step_narrowed = RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped");
        let error = standard()
            .tool_with_scope(step_narrowed)
            .run("second", &[], None)
            .await
            .expect_err("a run-approved program outside the step scope is refused");
        assert!(
            matches!(&error, RunnerError::ProgramNotAllowlisted { program } if program == "second"),
            "the refusal must be the typed not-allowlisted variant: {error}"
        );
        let outcome = standard()
            .tool_with_scope(run_approved)
            .run(PROGRAM, &["exit".to_owned(), "0".to_owned()], Some(30))
            .await
            .expect("the step's own program runs");
        assert_eq!(outcome.exit_code, Some(0));
    }

    #[tokio::test]
    async fn cwd_is_pinned_to_the_run_workspace() {
        let outcome = standard()
            .tool()
            .run(PROGRAM, &["cwd".to_owned()], Some(30))
            .await
            .expect("the cwd dumper must run");
        assert_eq!(
            outcome.stdout.text.trim(),
            standard().workspace.display().to_string(),
            "the child's cwd is the run workspace"
        );
    }

    #[tokio::test]
    async fn an_exit_code_is_data_not_an_error() {
        let outcome = standard()
            .tool()
            .run(PROGRAM, &["exit".to_owned(), "7".to_owned()], Some(30))
            .await
            .expect("a non-zero exit is an Ok outcome");
        assert_eq!(outcome.exit_code, Some(7));
        assert!(!outcome.killed_by_timeout);
    }

    #[tokio::test]
    async fn a_requested_timeout_narrows_but_never_widens() {
        let error = standard()
            .tool()
            .run(PROGRAM, &["sleep".to_owned(), "1".to_owned()], Some(301))
            .await
            .expect_err("a request above the default is refused");
        assert!(
            matches!(
                &error,
                RunnerError::TimeoutExceedsDefault {
                    requested: 301,
                    default: 300
                }
            ),
            "the refusal must carry both numbers: {error}"
        );
        let outcome = standard()
            .tool()
            .run(PROGRAM, &["sleep".to_owned(), "1".to_owned()], Some(30))
            .await
            .expect("a request at or below the default narrows");
        assert_eq!(outcome.exit_code, Some(0));
    }

    /// Cancellation kills the process group with the same killpg path the
    /// timeout uses, and the outcome says it was cancelled.
    #[tokio::test]
    async fn cancellation_kills_the_process_group() {
        let token = saya_agent::CancellationToken::new();
        let tool = standard().tool().with_cancellation(token.clone());
        let run = tokio::spawn(async move {
            tool.run(PROGRAM, &["sleep".to_owned(), "30".to_owned()], None)
                .await
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        token.cancel();
        let outcome = run
            .await
            .expect("the run task must finish")
            .expect("cancel reports the child");
        assert!(
            outcome.cancelled,
            "the cancellation must be reported: {outcome:?}"
        );
        assert!(
            !outcome.killed_by_timeout,
            "a cancelled child is not a timed-out child: {outcome:?}"
        );
        assert!(wait_group_gone(outcome.pid), "the group must be gone");
    }

    /// The runner is absent where the probe did not prove the sandbox —
    /// the M5-3 decision, consumed: no spawn configuration, no tool.
    #[test]
    fn the_runner_is_absent_where_the_probe_did_not_prove() {
        let workspace = leak("absent-ws");
        let programs = leak("absent-programs");
        let policy = RunSandbox::new([workspace.clone()], Vec::<(String, u16)>::new())
            .expect("the policy constructs while the root exists");
        fs::remove_dir_all(&workspace).expect("the root vanishes before prepare");
        let provision = policy.prepare(&programs).expect("preparation runs");
        assert!(!provision.report().proves_runner());
        assert!(provision.spawn().is_none(), "no spawn configuration exists");
    }
}
