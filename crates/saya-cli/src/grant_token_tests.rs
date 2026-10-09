//! The grant token suggestion: the bridge from one tool call to the
//! narrowest `--allow` grammar token a session grant for it would record.
//! Every token the suggester can produce must be a word the existing
//! grammar parser accepts — the parser is the authority, never a duplicate
//! of it — and a call the slice cannot name must yield `None`, which means
//! the tool keeps asking every call.

use crate::agent::tools::DatabaseTools;
use crate::approval_facts::{ApprovalFacts, RunnerFacts};
use crate::commands::run::scopes;
use crate::connection::ConnectionRegistry;
use crate::grant_token::{SQL_FAMILY, grant_token, session_answers_line};
use async_trait::async_trait;
use saya_agent::{ApprovalDecision, ApprovalPolicy, LocalStateEffect, SessionPolicy};
use saya_connectors::DatabaseConnector;
use saya_harness::fetch::FetchDestination;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use serde_json::{Value, json};
use std::path::PathBuf;

include!("grant_token_tests/support.rs");

include!("grant_token_tests/tokens.rs");
include!("grant_token_tests/connections.rs");
include!("grant_token_tests/workspace.rs");
include!("grant_token_tests/runner.rs");
