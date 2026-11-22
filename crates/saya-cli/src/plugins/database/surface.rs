//! The two surfaces the plugin serves: the chat's whole non-workspace toolset
//! over the database budget its turns are bounded by, and a coding job's
//! read-only pair over the authority view its consent selected. The
//! constructors live here so the plugin keeps that difference in one place.

use std::sync::{Arc, Mutex};

use saya_harness::coding::policy::ChildAuthorityView;

use super::prompt::ChatPrompt;
use super::{DatabaseBudget, DatabasePlugin};
use crate::agent::tools::{CaptureHook, DatabaseTools};
use crate::commands::coding::database::CodingDatabaseTools;

/// The gates the chat surface passes to `DatabaseTools::definitions`. The
/// workspace-write flag is not here: the workspace tools are the core's, so
/// this plugin always asks for their absent form.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChatDatabaseFlags {
    pub(crate) allow_query_data: bool,
    pub(crate) has_state_store: bool,
    pub(crate) permit_candidate_writes: bool,
}

/// The chat surface's toolset, the gates it advertises under, the database
/// budget a turn of this run spends once it uses the database, and the prompt
/// inputs the run handed the plugin once.
pub(super) struct ChatSurface {
    pub(super) tools: Arc<DatabaseTools>,
    pub(super) flags: ChatDatabaseFlags,
    pub(super) budget: DatabaseBudget,
    pub(super) prompt: ChatPrompt,
}

/// A coding job's toolset and the authority view it serves.
pub(super) struct JobSurface {
    pub(super) tools: Arc<CodingDatabaseTools>,
    pub(super) view: Option<ChildAuthorityView>,
}

/// Which surface's tools this plugin was built for, and the narrowed toolset it
/// serves them from. The job side is boxed: a `ChildAuthorityView` is far larger
/// than the chat side's flags, and the plugin is not a hot path that wants the
/// larger shape inline.
pub(super) enum Surface {
    Chat(ChatSurface),
    Job(Box<JobSurface>),
}

impl DatabasePlugin {
    /// The chat surface's database tools, over the database budget a turn of
    /// this run spends once it uses the database, plus the run's query-capture
    /// hook when the front drives one. The executor installs a hook on the
    /// tools either way: the front's own (when it drives one) behind the
    /// record that the next turn's last-SQL hint reads
    /// ([`super::DatabasePlugin::record_last_sql`]'s one live caller).
    pub(crate) fn for_chat(
        tools: Arc<DatabaseTools>,
        flags: ChatDatabaseFlags,
        budget: DatabaseBudget,
        prompt: ChatPrompt,
        capture: Option<CaptureHook>,
    ) -> Self {
        Self {
            surface: Surface::Chat(ChatSurface {
                tools,
                flags,
                budget,
                prompt,
            }),
            capture,
            recall: None,
            learning: None,
            last_receipt: Mutex::new(None),
            last_sql: Arc::new(Mutex::new(None)),
        }
    }

    /// A coding job's database tools, narrowed to the profiles `view` admits.
    /// No capture hook: a job's queries are its own evidence, never the
    /// front's preview.
    pub(crate) fn for_job(
        tools: Arc<CodingDatabaseTools>,
        view: Option<ChildAuthorityView>,
    ) -> Self {
        Self {
            surface: Surface::Job(Box::new(JobSurface { tools, view })),
            capture: None,
            recall: None,
            learning: None,
            last_receipt: Mutex::new(None),
            last_sql: Arc::new(Mutex::new(None)),
        }
    }

    /// Test-only seam: attaches a capture hook to a surface that publishes
    /// none, so a test can prove the job surface routes nothing to it.
    #[cfg(test)]
    pub(crate) fn with_test_capture(mut self, capture: Option<CaptureHook>) -> Self {
        self.capture = capture;
        self
    }
}
