/// The fully composed session shape: every surviving member carried —
/// every family the grammar grants on the session surface exists in this
/// composition. What the suggestion gates on is the composed universe,
/// never the call's arguments alone.
fn composed_facts() -> ApprovalFacts {
    ApprovalFacts {
        workspace_root: Some(PathBuf::from("/home/user/proj")),
        fetch: Some(crate::approval_facts::FetchFacts {
            fetch_body_bytes: 61_440,
            fetch_seconds: 30,
            fetch_redirects: 5,
            download: None,
        }),
        ..ApprovalFacts::default()
    }
}

/// The token a call suggests, for the tests that need it as a value. The
/// suggester reads the turn's primary through the third argument and the
/// composition through the fourth; `None` there means none is bound or
/// composed.
fn token_for(tool: &str, arguments: Value, facts: &ApprovalFacts) -> String {
    grant_token(tool, &arguments, None, facts)
        .unwrap_or_else(|| panic!("{tool} with {arguments} must suggest a token for this test"))
}

/// The suggester's answer for one SQL-family call shape.
fn sql_suggestion(tool: &str, arguments: Value, registry: &ConnectionRegistry) -> Option<String> {
    grant_token(
        tool,
        &arguments,
        registry.primary(),
        &ApprovalFacts::default(),
    )
}

/// True when the parsed approval actually contains what `token` names —
/// the parse alone is not enough, the capability must be the named one.
/// A `sql:` token names no run capability: on the session surface it is
/// carried as the grant store's word, so the round-trip is the verbatim
/// token.
fn approves_what_it_names(token: &str, approved: &scopes::Approved) -> bool {
    if token.starts_with("sql:") {
        return approved.tokens.iter().any(|stated| stated == token);
    }
    let capabilities = &approved.capabilities;
    match token {
        "workspace-write" => capabilities.workspace_write,
        other => match other.split_once(':') {
            Some(("fetch", rest)) => capabilities.fetch.as_ref().is_some_and(|scope| {
                scope
                    .destinations
                    .iter()
                    .any(|d| format!("{}+{}", d.scheme, d.host) == rest)
            }),
            _ => false,
        },
    }
}

struct DummyConnector {
    dialect: SqlDialect,
}

#[async_trait]
impl DatabaseConnector for DummyConnector {
    fn dialect(&self) -> SqlDialect {
        self.dialect
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Err(ConnectionError::schema_failed("dummy"))
    }

    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::query_failed("dummy"))
    }
}

/// A registry whose primary named `name` is connected — the shape the turn
/// builder produces when a profile is selected. Shared with the approval
/// tests, which bind it into a decider's primary handle.
pub(crate) fn registry_with_primary(name: &str) -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new(name);
    registry.insert(
        name,
        crate::connection::ConnectionEntry {
            connector: Box::new(DummyConnector {
                dialect: SqlDialect::Postgres,
            }),
            dialect: SqlDialect::Postgres,
            profile_id: None,
        },
    );
    registry
}

/// A registry with no connection — the shape the turn builder produces when
/// no profile is selected (`ConnectionRegistry::new("")`, nothing inserted).
fn empty_registry() -> ConnectionRegistry {
    ConnectionRegistry::new("")
}
