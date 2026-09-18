use crate::interactive::tui::types::SessionUsage;
use saya_agent::{ChatMessage, ToolMetadata};
use saya_store::{
    RedactedSession, RedactedToolMetadata, RedactedToolResultShape, RedactedTurn, SESSION_VERSION,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionState {
    pub id: String,
    pub profile: Option<String>,
    pub included_profiles: Vec<String>,
    pub provider: String,
    pub model: String,
    pub allow_data_sharing: bool,
    pub approval_mode: String,
    /// The session's task posture (`AgentMode::as_str` spelling), beside
    /// `approval_mode`: `build` does the work, `plan` investigates read-only.
    /// Carried as the mode's own string so the persisted session file stays
    /// a plain string like `approval_mode`; parsed at the composition root.
    /// Defaults to Build; `#[serde(default)]` keeps old session files
    /// loading (they predate the field and resume as build).
    #[serde(default = "default_agent_mode")]
    pub agent_mode: String,
    /// The session's pinned workspace root, canonical (SESSION-WORKSPACE.md):
    /// resolved once at first start — the git worktree top, or an explicit
    /// `--workspace` — persisted here, and re-opened on a resume, never
    /// re-derived from wherever the shell happens to be. `None` binds no
    /// root: the write-shaped tools stay hidden and workspace reads refuse
    /// with their typed error, which is every session written before the
    /// workspace existed.
    #[serde(default)]
    pub workspace_root: Option<String>,
    /// The session's task list: conversation metadata tracking what the
    /// session is building, binding no authority. Modelled on `agent_mode`:
    /// an additive `#[serde(default)]` field, so a session file written
    /// before it existed resumes with an empty list and no `SESSION_VERSION`
    /// bump is owed.
    #[serde(default)]
    pub task_list: saya_types::SessionTaskList,
    pub messages: Vec<SessionLine>,
    pub turns: Vec<RedactedTurn>,
    /// The `/compact` summary of the older turns, replayed ahead of the
    /// verbatim tail. Working memory only (`#[serde(skip)]`, the `usage`
    /// precedent): the transcript keeps what was said, compaction rewrites
    /// only what the model replays. `/clear` resets it with the rest of the
    /// conversation.
    #[serde(skip)]
    pub compaction_summary: Option<String>,
    /// How many oldest turns the summary replaces. `#[serde(skip)]` with the
    /// summary: a resumed session replays its persisted turns in full.
    #[serde(skip)]
    pub compacted_turns: usize,
    /// Whether the model's chain-of-thought is shown in the transcript. Off by
    /// default; toggled by `/thinking` or `--show-thinking`. In-memory only: it
    /// is a display preference, not conversation data. A resumed session
    /// deserializes it as off and the startup path re-derives it from the
    /// config setting and `--show-thinking`. `#[serde(skip)]` keeps it out of
    /// persisted session files.
    #[serde(skip)]
    pub show_thinking: bool,
    /// In-memory session token accumulator. `#[serde(skip)]` keeps it
    /// out of persisted session files (no new persisted state);
    /// a resumed session starts with a fresh total. `/clear` resets it.
    #[serde(skip)]
    pub(crate) usage: SessionUsage,
    /// Whether the host-command lane composed for this session: the status
    /// header's `host:` segment reads this. In-memory only — a resumed
    /// session recomposes the lane from its launch statement, never from
    /// the record — so `#[serde(skip)]` keeps it out of persisted files.
    #[serde(skip)]
    pub host_composed: bool,
    /// Whether the context-window warning already fired for the current
    /// above-threshold stretch. Set on the upward crossing of
    /// `CONTEXT_WARN_PERCENT`, cleared when utilisation falls back below it
    /// and by `/clear`. In-memory only (`#[serde(skip)]`, the `show_thinking`
    /// precedent): a resumed session re-derives it from the next turn's
    /// usage rather than persisting a stale flag.
    #[serde(skip, default = "default_context_warned")]
    pub context_warned: bool,
    /// Whether an automatic compaction already tried and failed this session.
    /// Set on an automatic failure, cleared by `/clear` and by any
    /// successful compaction (manual or automatic). In-memory only
    /// (`#[serde(skip)]`): a resumed session re-derives it from the next
    /// turn's outcome rather than persisting a stale refusal. While set the
    /// automatic trigger stays silent — a session that fails to summarise
    /// every turn forever would burn the budget it was trying to save — and
    /// only an explicit `/compact` (or a fresh context) re-arms it.
    #[serde(skip, default = "default_auto_compact_failed")]
    pub auto_compact_failed: bool,
    /// The session's deny list: bare program names every door refuses. The
    /// status header lists them. In-memory only — recomposed from the launch
    /// statement and user-layer config, never from the record.
    #[serde(skip)]
    pub denied_programs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionLine {
    pub role: String,
    pub content: String,
}
pub type Session = SessionState;

impl SessionState {
    pub fn new(id: impl Into<String>, profile: Option<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            profile,
            included_profiles: Vec::new(),
            provider: "ollama".into(),
            model: model.into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
            agent_mode: default_agent_mode(),
            workspace_root: None,
            task_list: saya_types::SessionTaskList::default(),
            messages: Vec::new(),
            turns: Vec::new(),
            compaction_summary: None,
            compacted_turns: 0,
            show_thinking: false,
            usage: SessionUsage::default(),
            host_composed: false,
            context_warned: default_context_warned(),
            auto_compact_failed: default_auto_compact_failed(),
            denied_programs: Vec::new(),
        }
    }

    pub fn record(&mut self, role: &str, content: impl Into<String>) {
        self.messages.push(SessionLine {
            role: role.into(),
            content: content.into(),
        });
    }

    pub fn record_turn(
        &mut self,
        user: impl Into<String>,
        assistant: impl Into<String>,
        database_derived: bool,
        tools: Vec<ToolMetadata>,
    ) {
        let user = user.into();
        let assistant = assistant.into();
        self.messages.push(SessionLine {
            role: "user".into(),
            content: user.clone(),
        });
        self.messages.push(SessionLine {
            role: "assistant".into(),
            content: assistant.clone(),
        });
        self.turns.push(RedactedTurn {
            user,
            assistant,
            database_derived,
            tools: tools
                .into_iter()
                .map(|tool| RedactedToolMetadata {
                    name: tool.name,
                    status: tool.status,
                    arguments: tool.arguments,
                    result_shape: tool.result_shape.map(|shape| RedactedToolResultShape {
                        row_count: shape.row_count,
                        columns: shape.columns,
                    }),
                })
                .collect(),
        });
    }

    pub fn provider_history(&self) -> Vec<ChatMessage> {
        let include_sensitive =
            self.provider.eq_ignore_ascii_case("ollama") || self.allow_data_sharing;
        let mut history: Vec<ChatMessage> = self
            .turns
            .iter()
            .filter(|turn| include_sensitive || !turn.database_derived)
            .flat_map(|turn| {
                [
                    ChatMessage::text("user", turn.user.clone()),
                    ChatMessage::text("assistant", turn.assistant.clone()),
                ]
            })
            .collect();
        // A `/compact` summary replays ahead of the verbatim tail it
        // replaced: the count was recorded at compaction time over the same
        // filtered view only when no turn was filtered out, so a filtered
        // session replays its kept turns in full rather than a misaligned
        // prefix. The transcript (`messages`) is untouched throughout.
        if let Some(summary) = self.compaction_summary.as_deref()
            && self.compacted_turns > 0
            && self
                .turns
                .iter()
                .all(|turn| include_sensitive || !turn.database_derived)
        {
            let drop = (self.compacted_turns * 2).min(history.len());
            history.drain(..drop);
            history.insert(0, ChatMessage::text("user", format!("Earlier conversation summary (compacted from {} turns; the newest turns below are verbatim):\n{summary}", self.compacted_turns)));
        }
        history
    }

    pub fn redacted(&self) -> RedactedSession {
        RedactedSession {
            version: SESSION_VERSION,
            id: self.id.clone(),
            profile: self.profile.clone(),
            included_profiles: self.included_profiles.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            allow_data_sharing: self.allow_data_sharing,
            approval_mode: self.approval_mode.clone(),
            agent_mode: self.agent_mode.clone(),
            workspace_root: self.workspace_root.clone(),
            task_list: self.task_list.clone(),
            turns: self.turns.clone(),
            profile_names: self.profile_names(),
            messages: Vec::new(),
        }
    }

    fn profile_names(&self) -> Vec<String> {
        self.profile
            .iter()
            .cloned()
            .chain(self.included_profiles.iter().cloned())
            .collect()
    }

    /// The session's task posture, parsed from the stored spelling. An
    /// unknown or legacy-absent value falls back to Build — the composition
    /// root reads this, never the raw string, so a corrupt or pre-`/mode`
    /// session still turns exactly as before.
    pub fn agent_mode_parsed(&self) -> saya_agent::AgentMode {
        self.agent_mode.parse().unwrap_or_default()
    }
}

/// The default task posture's own spelling: `AgentMode::Build`, rendered
/// where the mode is named. The `serde(default)` entry point for
/// `SessionState::agent_mode`, so session files written before the field
/// existed resume as build.
fn default_agent_mode() -> String {
    saya_agent::AgentMode::Build.as_str().into()
}

/// A deserialized session starts with the context warning armed: the flag is
/// `#[serde(skip)]`, so nothing on disk sets it, and an armed start means the
/// next turn's usage re-derives the flag from a live report.
fn default_context_warned() -> bool {
    false
}

/// A deserialized session starts with automatic compaction armed: the flag is
/// `#[serde(skip)]`, so nothing on disk sets it, and an armed start means the
/// next crossing may fire from a live report.
fn default_auto_compact_failed() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::ToolResultShape;

    /// A resumed session carries no stale warn flag: a fresh session starts
    /// armed, and an old session file (no such key) deserializes armed too,
    /// so the next turn's usage re-derives the flag from a live report.
    #[test]
    fn the_context_warn_flag_starts_armed_and_stays_out_of_persisted_sessions() {
        let fresh = SessionState::new("s1", None, "m");
        assert!(
            !fresh.context_warned,
            "a fresh session starts with the warning armed (unfired)"
        );
        let json = serde_json::to_string(&fresh).expect("serializes");
        assert!(
            !json.contains("context_warned"),
            "the flag must stay out of persisted sessions: {json}"
        );
        // An old session file predates the flag: it deserializes armed.
        let old = r#"{"id":"s1","profile":null,"included_profiles":[],"provider":"ollama","model":"m","allow_data_sharing":false,"approval_mode":"ask","agent_mode":"build","workspace_root":null,"messages":[],"turns":[]}"#;
        let loaded: SessionState = serde_json::from_str(old).expect("old form deserializes");
        assert!(
            !loaded.context_warned,
            "an old session file resumes with the warning armed (unfired)"
        );
    }

    /// `/clear` re-arms the context warning along with the conversation: the
    /// working memory is fresh, so the next crossing must warn again.
    #[test]
    fn clear_re_arms_the_context_warning() {
        let mut state = SessionState::new("s1", None, "m");
        state.context_warned = true;
        state.apply(crate::SlashCommand::Clear, &[]);
        assert!(
            !state.context_warned,
            "/clear must re-arm the warning, not leave it fired"
        );
    }

    /// A turn that carried chain-of-thought must leave none of it in a
    /// persisted session. The reasoning lives on `ChatResponse.reasoning`
    /// (transport for a single call) and on `AgentEvent::ReasoningText` (an
    /// in-memory event); neither has a field on the persisted types, and
    /// `record_turn` — the only write path into a session — takes `user`,
    /// `assistant`, `database_derived` and `tools`, so there is no argument a
    /// caller could pass the reasoning through.
    ///
    /// What this pins is the shape of the persisted form: no `reasoning` key
    /// appears on a serialized session. The companion test beside `apply_event`
    /// covers the stronger property — reasoning genuinely on screen and still
    /// absent from disk — which needs the transcript, unreachable from here.
    #[test]
    fn a_session_persisted_after_a_reasoning_turn_contains_none_of_it() {
        let reasoning = "the secret chain-of-thought about row values 9f3a";
        let mut session =
            SessionState::new("s1", Some(String::from("analytics")), String::from("m"));
        session.record_turn("what is the answer", "the answer is 42", false, Vec::new());

        let json = serde_json::to_string(&session).expect("serializes");
        assert!(
            !json.contains(reasoning),
            "reasoning leaked into the persisted session: {json}"
        );
        assert!(
            !json.contains("reasoning"),
            "a `reasoning` key appeared on a persisted session: {json}"
        );

        let replayed = session.provider_history();
        assert!(
            replayed.iter().all(|m| !m.content.contains(reasoning)),
            "reasoning leaked into replayed history: {replayed:?}"
        );
    }

    /// A turn that ran SQL persists the statement (in `arguments`) and the
    /// value-free result shape (row count + column names). This is the core of
    /// "let a session reproduce its own run": tomorrow a user can answer "what
    /// did it actually do?" from the session file alone.
    #[test]
    fn a_session_that_ran_sql_stores_the_statement_and_shape() {
        let mut session = SessionState::new("s1", None, "m");
        session.record_turn(
            "count customers",
            "42 customers",
            true,
            vec![ToolMetadata {
                name: "bounded_sql_query".into(),
                status: "completed".into(),
                arguments: r#"{"sql":"SELECT customer, age FROM users"}"#.into(),
                result_shape: Some(ToolResultShape {
                    row_count: 1,
                    columns: vec!["customer".into(), "age".into()],
                }),
            }],
        );

        let saved = session.redacted();
        let tool = &saved.turns[0].tools[0];
        assert_eq!(tool.name, "bounded_sql_query");
        assert_eq!(tool.status, "completed");
        assert!(
            tool.arguments.contains("SELECT customer, age FROM users"),
            "the statement must be persisted: {}",
            tool.arguments
        );
        let shape = tool
            .result_shape
            .as_ref()
            .expect("the result shape is persisted");
        assert_eq!(shape.row_count, 1);
        assert_eq!(shape.columns, vec!["customer", "age"]);
    }

    /// A cell value present in a live query result never reaches the session
    /// file. The persisted record carries the statement and the value-free
    /// shape (row count + column names) only; `rows` is never stored. The
    /// agent's `result_shape_of` test pins that the shape excludes cells; this
    /// test pins the session-file half: write a session whose tool record
    /// mirrors that shape and assert the cell is absent and no `rows` key
    /// appears.
    #[test]
    fn a_result_cell_value_never_reaches_the_session_file() {
        use saya_store::{FsSessionStore, SessionStore};
        // A live query result carrying a cell value. The agent reads only its
        // row_count and columns (pinned in saya-agent's `result_shape_of`
        // test); the record below mirrors that shape, with no rows.
        let live_result = serde_json::json!({
            "columns": ["customer"],
            "rows": [["CELL_SENTINEL_9f3a"]],
            "row_count": 1,
            "truncated": false
        });
        let shape = ToolResultShape {
            row_count: live_result["row_count"].as_u64().unwrap(),
            columns: live_result["columns"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| name.as_str().unwrap().to_owned())
                .collect(),
        };
        let mut session = SessionState::new("cell-test", None, "m");
        session.record_turn(
            "show customers",
            "here they are",
            true,
            vec![ToolMetadata {
                name: "bounded_sql_query".into(),
                status: "completed".into(),
                arguments: r#"{"sql":"SELECT customer FROM users"}"#.into(),
                result_shape: Some(shape),
            }],
        );
        let root = std::env::temp_dir().join(format!("saya-cell-{}", std::process::id()));
        let store = FsSessionStore::new(&root);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(store.save(session.redacted())).unwrap();
        let file = std::fs::read_to_string(root.join("cell-test.json")).unwrap();
        assert!(
            !file.contains("CELL_SENTINEL_9f3a"),
            "a cell value reached the session file: {file}"
        );
        assert!(
            !file.contains("\"rows\""),
            "a `rows` key appeared in a tool record: {file}"
        );
        // Round-trip through the store and confirm the persisted record keeps
        // the statement and the value-free shape, independent of pretty-print
        // spacing.
        let loaded = runtime
            .block_on(store.load("cell-test"))
            .unwrap()
            .expect("the session file loads");
        let tool = &loaded.turns[0].tools[0];
        assert!(
            tool.arguments.contains("SELECT customer FROM users"),
            "the statement must be persisted: {}",
            tool.arguments
        );
        let shape = tool.result_shape.as_ref().expect("the shape is persisted");
        assert_eq!(shape.row_count, 1);
        assert_eq!(shape.columns, vec!["customer"]);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A resumed session's replayed history carries no tool calls or
    /// tool-call ids, so the agent's history validator (which rejects both)
    /// accepts it on every provider — the persisted tool record lives beside
    /// the conversation, not inside the replayed messages. This is why
    /// `validate()` is left unchanged.
    #[test]
    fn a_resumed_session_with_tool_calls_produces_valid_history_for_every_provider() {
        let mut session = SessionState::new("s1", None, "m");
        session.record_turn(
            "q",
            "a",
            true,
            vec![ToolMetadata {
                name: "bounded_sql_query".into(),
                status: "completed".into(),
                arguments: r#"{"sql":"SELECT 1"}"#.into(),
                result_shape: Some(ToolResultShape {
                    row_count: 1,
                    columns: vec!["c".into()],
                }),
            }],
        );
        let history = session.provider_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[1].role, "assistant");
        for message in &history {
            assert!(
                message.tool_calls.is_empty(),
                "replayed history carried tool_calls: {message:?}"
            );
            assert!(
                message.tool_call_id.is_none(),
                "replayed history carried a tool_call_id: {message:?}"
            );
        }
    }

    /// The persisted mode carries the task posture and nothing secret: `plan`
    /// round-trips through `redacted()`, and the serialized form carries the
    /// mode word with no secret-shaped keys beside the ones the redaction
    /// tests already pin. The task list rides the same record: it round-trips
    /// too, and its titles and notes — plain conversation metadata — add no
    /// new secret-shaped key.
    #[test]
    fn redacted_carries_the_mode_and_no_secret_shaped_data() {
        use saya_agent::AgentMode;
        use saya_types::{SessionTask, SessionTaskList, TaskStatus};
        let mut planned = SessionState::new("s-mode", None, "m");
        planned.agent_mode = AgentMode::Plan.as_str().into();
        planned.task_list = SessionTaskList::new(vec![
            SessionTask::with_note(
                "profile the tables",
                TaskStatus::InProgress,
                Some("halfway"),
            )
            .unwrap(),
        ])
        .unwrap();
        let saved = planned.redacted();
        assert_eq!(saved.agent_mode, "plan");
        assert_eq!(saved.approval_mode, "ask");
        assert_eq!(saved.task_list, planned.task_list);
        let json = serde_json::to_string(&saved).unwrap();
        assert!(
            json.contains(r#""agent_mode":"plan""#),
            "mode missing: {json}"
        );
        assert!(
            json.contains("profile the tables"),
            "the task list must ride the record: {json}"
        );
        for key in ["password", "api_key", "rows", "reasoning"] {
            assert!(
                !json.contains(key),
                "`{key}` leaked into the record: {json}"
            );
        }

        let built = SessionState::new("s-build", None, "m");
        assert_eq!(built.redacted().agent_mode, "build");
        assert!(
            built.redacted().task_list.is_empty(),
            "a fresh session persists an empty list"
        );
    }

    /// A session with no tool calls serializes to today's shape: no
    /// `arguments` or `result_shape` keys appear, and the turn is just user +
    /// assistant. A tool record with empty arguments and no shape is also
    /// byte-compatible with a file written before this feature (the new keys
    /// are `skip_serializing_if`-omitted), so old and new files interoperate.
    #[test]
    fn a_session_with_no_tool_calls_is_unchanged_from_today() {
        let mut session = SessionState::new("s1", None, "m");
        session.record_turn("hello", "hi there", false, vec![]);
        let json = serde_json::to_string(&session.redacted()).unwrap();
        assert!(!json.contains("arguments"), "new key leaked: {json}");
        assert!(!json.contains("result_shape"), "new key leaked: {json}");
        assert!(
            json.contains(r#""user":"hello""#),
            "user turn missing: {json}"
        );
        assert!(
            json.contains(r#""assistant":"hi there""#),
            "assistant turn missing: {json}"
        );

        // A tool record with no extra data serializes as name + status only.
        let mut with_tool = SessionState::new("s2", None, "m");
        with_tool.record_turn(
            "q",
            "a",
            true,
            vec![ToolMetadata {
                name: "schema_discovery".into(),
                status: "completed".into(),
                ..Default::default()
            }],
        );
        let tool_json = serde_json::to_string(&with_tool.redacted()).unwrap();
        assert!(
            tool_json.contains(r#""tools":[{"name":"schema_discovery","status":"completed"}]"#),
            "a tool record with no extra data should be name + status only: {tool_json}"
        );
        assert!(
            !tool_json.contains("arguments") && !tool_json.contains("result_shape"),
            "empty optional fields leaked into the wire form: {tool_json}"
        );
    }
}
