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
