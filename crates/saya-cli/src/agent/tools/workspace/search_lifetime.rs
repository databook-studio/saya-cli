//! Bounded workspace workers remain owned until their blocking work drains.
use super::super::DatabaseTools;
use super::grep_types;
use saya_agent::{CancellationReason, CancellationToken, ToolError, ToolExecutionContext};
use saya_harness::workspace::{
    GrepOptions, SearchEntry, SearchError, SearchKind, SearchLimits, SearchMatch, SearchOutput,
    SearchQuery,
};
use serde_json::Value;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[derive(Default)]
pub(in crate::agent::tools) struct WorkspaceTasks {
    active: AtomicUsize,
    drained: Notify,
}
const MAX_ACTIVE_WORKERS: usize = 4;

impl WorkspaceTasks {
    pub(in crate::agent::tools) fn begin(self: &Arc<Self>) -> Result<WorkerGuard, ToolError> {
        let mut active = self.active.load(Ordering::Acquire);
        loop {
            if active >= MAX_ACTIVE_WORKERS {
                return Err(refusal("workspace worker owner is busy"));
            }
            match self.active.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => active = current,
            }
        }
        Ok(WorkerGuard(Arc::clone(self)))
    }
    pub(in crate::agent::tools) async fn settle(&self) {
        loop {
            let notified = self.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.active.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
}
pub(in crate::agent::tools) struct WorkerGuard(Arc<WorkspaceTasks>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if self.0.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.drained.notify_waiters();
        }
    }
}
pub(in crate::agent::tools) struct CancellationOnDrop(Option<CancellationToken>);

impl CancellationOnDrop {
    pub(in crate::agent::tools) fn new(token: CancellationToken) -> Self {
        Self(Some(token))
    }

    pub(in crate::agent::tools) fn disarm(&mut self) {
        self.0.take();
    }
}

impl Drop for CancellationOnDrop {
    fn drop(&mut self) {
        if let Some(token) = self.0.take() {
            token.cancel_with_reason(CancellationReason::OwnerFailure);
        }
    }
}
pub(super) fn refusal(message: &'static str) -> ToolError {
    ToolError::NoEffect(Box::new(ToolError::Runner(message.into())))
}

pub(in crate::agent::tools) fn validate_search(
    name: &str,
    arguments: &Value,
    context: &ToolExecutionContext,
) -> Result<(), ToolError> {
    super::validation::validate_arguments(name, arguments)?;
    validate_context(context, "workspace search")
}

pub(in crate::agent::tools) fn validate_context(
    context: &ToolExecutionContext,
    operation: &'static str,
) -> Result<(), ToolError> {
    if context.result_cap < 512 {
        return Err(refusal(
            "workspace result cap cannot hold truthful metadata",
        ));
    }
    if context.cancellation.is_cancelled() {
        return Err(refusal(match operation {
            "workspace read" => "workspace read was cancelled before dispatch",
            "workspace list" => "workspace list was cancelled before dispatch",
            _ => "workspace search was cancelled before dispatch",
        }));
    }
    Ok(())
}

/// Rewrites spool-relative result paths to the absolute spelling `read`
/// accepts, so a match the model sees is a path it can open.
fn name_spool_matches(matches: &mut [SearchMatch], base: &Path) {
    for entry in matches {
        let path = match entry {
            SearchMatch::Path(path)
            | SearchMatch::Line { path, .. }
            | SearchMatch::Count { path, .. } => path,
        };
        *path = base.join(&*path).display().to_string();
    }
}

pub(in crate::agent::tools) async fn execute_search(
    tools: &DatabaseTools,
    name: &str,
    arguments: &Value,
    context: ToolExecutionContext,
    private_prefixes: Vec<String>,
) -> Result<Value, ToolError> {
    validate_search(name, arguments, &context)?;
    let workspace = Arc::clone(
        tools
            .workspace
            .as_ref()
            .ok_or(ToolError::WorkspaceUnavailable)?,
    );
    let pattern = arguments["pattern"]
        .as_str()
        .ok_or(ToolError::PatternNotString)?;
    let regex = arguments
        .get("regex")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // An absolute path under the workspace root is the same directory as its
    // relative spelling. Any other path is left for the harness to refuse, so
    // the refusal keeps its existing shape.
    let stated = arguments.get("path").and_then(Value::as_str).unwrap_or("");
    // This job's own shell-output spool is the one outside tree `grep`
    // reaches. It is opened as its own workspace, and every served path is
    // rewritten to the absolute spelling `read` accepts back.
    let (workspace, path, spool_base) = match tools.spool_target(stated) {
        Some(target) => (
            Arc::new(target.workspace),
            target.relative,
            Some(target.base),
        ),
        None => {
            let path = workspace
                .relative_argument(stated)
                .map_or_else(|_| stated.to_owned(), |relative| relative.into_owned());
            (workspace, path, None)
        }
    };
    let query = SearchQuery {
        kind: if name == "glob" {
            SearchKind::Glob
        } else {
            SearchKind::Grep
        },
        pattern: pattern.into(),
        path,
        include: arguments
            .get("include")
            .and_then(Value::as_str)
            .map(str::to_owned),
        regex,
        case_insensitive: arguments
            .get("case_insensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        // Only glob's `type` selects an entry kind; validation admits `file`
        // and `dir` and grep reads its own `type` as a language below.
        entry: match arguments.get("type").and_then(Value::as_str) {
            Some("file") => Some(SearchEntry::File),
            Some("dir") => Some(SearchEntry::Dir),
            _ => None,
        },
        // Validation admits only the three modes, a `type` inside the bounded
        // map, and the bounded numbers; whatever reaches here is in range.
        grep: GrepOptions {
            mode: match arguments.get("output_mode").and_then(Value::as_str) {
                Some("files_with_matches") => SearchOutput::FilesWithMatches,
                Some("count") => SearchOutput::Count,
                _ => SearchOutput::Content,
            },
            context: arguments
                .get("context")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
            multiline: arguments
                .get("multiline")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            head_limit: arguments
                .get("head_limit")
                .and_then(Value::as_u64)
                .map(|limit| limit as usize),
            extensions: arguments
                .get("type")
                .and_then(Value::as_str)
                .and_then(grep_types::lookup)
                .map(|entry| {
                    entry
                        .extensions
                        .iter()
                        .map(|extension| (*extension).to_string())
                        .collect()
                }),
        },
        private_prefixes,
    };
    let limits = SearchLimits {
        result_bytes: context.result_cap.min(48 * 1024),
        ..SearchLimits::default()
    };
    let guard = tools.workspace_tasks.begin()?;
    let cancellation = context.cancellation;
    let mut stop = CancellationOnDrop::new(cancellation.clone());
    let worker = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        workspace.search_controlled(&query, &limits, &cancellation)
    });
    let result = worker.await;
    stop.disarm();
    let mut outcome = result
        .map_err(|_| ToolError::Runner("workspace search worker outcome is unavailable".into()))?
        .map_err(|error| match error {
            SearchError::Invalid => refusal("invalid bounded workspace search arguments"),
            SearchError::Cancelled => ToolError::Runner("workspace search was cancelled".into()),
        })?;
    if let Some(base) = spool_base {
        name_spool_matches(&mut outcome.matches, &base);
    }
    let empty = outcome.matches.is_empty() && outcome.matches_found == 0;
    let mut value = serde_json::to_value(outcome)
        .map_err(|_| ToolError::Runner("workspace search result is unavailable".into()))?;
    // A literal miss on a pattern full of regex syntax is the silent wrong
    // conclusion this note prevents; the default stays literal.
    if name == "grep" && empty && !regex && grep_types::has_regex_metacharacters(pattern) {
        value["note"] = Value::String(
            "no literal match; this pattern contains regex metacharacters — pass `regex: true` \
             to search as a regex"
                .into(),
        );
    }
    Ok(value)
}
