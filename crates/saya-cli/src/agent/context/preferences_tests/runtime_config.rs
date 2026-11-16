struct IdleConnector;

#[async_trait]
impl DatabaseConnector for IdleConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree {
            databases: vec![Database {
                name: "catalog".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables: vec![Table {
                        name: "orders".into(),
                        columns: Vec::new(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                    }],
                }],
            }],
        })
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

fn registry_for(name: &str) -> ConnectionRegistry {
    let identity: ProfileIdentity = crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::Sqlite {
            path: "preference-context.sqlite".into(),
            read_only: true,
        },
        Path::new("/preference-context-test/connections.toml"),
    );
    let mut registry = ConnectionRegistry::new(name);
    registry.insert(
        name,
        ConnectionEntry {
            connector: Box::new(IdleConnector),
            dialect: SqlDialect::Sqlite,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    registry
}

fn ai_for(context_byte_budget: usize, privacy_gate_closed: bool) -> ResolvedAi {
    let mut ai = ResolvedAi {
        app_attribution: true,
        provider: AiProvider::Ollama,
        model: "test-model".into(),
        base_url: None,
        api_key: None,
        allow_data_sharing: true,
        temperature: 0.0,
        timeout_seconds: 60,
        idle_timeout_seconds: 90,
        request_timeout_seconds: 180,
        max_output_tokens: None,
        reasoning_effort: None,
        context_byte_budget,
        context_window_tokens: None,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        compaction_trigger_percent: 50,
        retry_delays_ms: vec![250, 500, 1000],
    };
    if privacy_gate_closed {
        // A non-local provider with sharing off: `allow_query_data` computes
        // false for the turn. Preferences must still ride.
        ai.provider = AiProvider::Openai;
        ai.allow_data_sharing = false;
    }
    ai
}

fn runtime_config(context_byte_budget: usize, privacy_gate_closed: bool) -> RuntimeConfig {
    RuntimeConfig {
        resolved: ResolvedConfig {
            hooks: Default::default(),
            tools: Default::default(),
            web_search: Default::default(),
            mcp_servers: Default::default(),
            permissions: Default::default(),
            coding: Default::default(),
            agents: Default::default(),
            instructions: saya_config::ResolvedInstructions {
                project_enabled: true,
                personal_enabled: false,
                project_max_bytes: 32 * 1024,
                ..saya_config::ResolvedInstructions::default()
            },
            profile_name: None,
            profile: None,
            limits: Default::default(),
            ai: ai_for(context_byte_budget, privacy_gate_closed),
            max_rows: 100,
            read_only: true,
            candidates: 1,
            jobs: ResolvedJobs::default(),
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            ui_theme: ThemeChoice::Auto,
            memory: ResolvedMemory {
                mode: MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            provenance: Default::default(),
            endpoints: std::collections::BTreeMap::new(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-preference-context-tests"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: std::collections::BTreeMap::new(),
    }
}
