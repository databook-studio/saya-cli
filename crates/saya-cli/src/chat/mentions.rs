//! `@path` mentions: the paths the composer offers, and the files a message
//! carries.
//!
//! One reading, two surfaces. The full-screen surface offers the paths as a
//! list above the composer and inserts the one the operator picks; the line
//! front has no list and only expands what was typed. Both read a mention the
//! same way, and both attach a mentioned file the same way, because this module
//! is the only place either is decided.
//!
//! **The paths are the file tools' own listing.** [`candidates`] runs the
//! bounded glob walk the `glob` tool runs, so a path the tool would not list —
//! ignored by `.gitignore`, private by the shared privacy rule — is a path the
//! popup never offers. The list is bounded by entries and by time.
//!
//! **A mentioned file is read the way the `read` tool reads it.** [`attach`]
//! opens the run's contained workspace and reads each named path through the
//! same bounded read: [`MAX_FILE_BYTES`] per file, [`MAX_TOTAL_BYTES`] for all
//! of them, at most [`MAX_MENTIONS`] files. A file over the per-file bound is
//! not truncated into the message — it is named with its size, so the model
//! knows to narrow with a ranged read. A path the file tools refuse is refused
//! with the read tool's own sentence.
//!
//! The model need not call `read` first: what the operator pointed at is in
//! the message already, which is the whole point of the mention.

use std::time::Duration;

use saya_harness::workspace::{
    SearchEntry, SearchKind, SearchLimits, SearchMatch, SearchQuery, Workspace,
};

/// The most files one message carries, however many it mentions.
pub(crate) const MAX_MENTIONS: usize = 8;

/// The most bytes one mentioned file contributes to the message.
pub(crate) const MAX_FILE_BYTES: u64 = 16 * 1024;

/// The most bytes every mentioned file contributes together.
pub(crate) const MAX_TOTAL_BYTES: usize = 64 * 1024;

/// The most paths one listing returns, and the walk's own entry bound. The
/// walk's match cap is the harness's own ceiling — below this — so the listing
/// is what the walk reached, never a bound this module invents.
pub(crate) const MAX_LISTED: usize = 1_000;
/// The walk's match cap: the harness's ceiling on one search's matches.
const MATCH_CAP: usize = 500;

/// How long the listing may walk before it keeps what it reached.
const LIST_BUDGET: Duration = Duration::from_millis(300);

/// The line that introduces what a message carries.
pub(crate) const HEADER: &str = "--- mentioned files ---";

/// The `@path` tokens `message` names, in the order they appear, without the
/// `@` and without repeats, bounded to [`MAX_MENTIONS`].
pub(crate) fn mentioned(message: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for token in message.split_whitespace() {
        let Some(path) = token.strip_prefix('@') else {
            continue;
        };
        let path = path.trim_end_matches([',', ';', ':', '.', ')', ']', '}', '?', '!']);
        if path.is_empty() || paths.iter().any(|seen| seen == path) {
            continue;
        }
        if paths.len() == MAX_MENTIONS {
            break;
        }
        paths.push(path.to_owned());
    }
    paths
}

/// The path being typed after an `@` at the end of `text`, when there is one:
/// the popup is open exactly while this answers.
pub(crate) fn query(text: &str) -> Option<&str> {
    let start = text
        .char_indices()
        .rev()
        .take_while(|(_, character)| !character.is_whitespace())
        .last()
        .map(|(index, _)| index)?;
    let token = &text[start..];
    let path = token.strip_prefix('@')?;
    match start == 0 {
        true => Some(path),
        // A mention starts a word: `mail@saya.dev` is an address, not a path.
        false => {
            let before = text[..start].chars().next_back()?;
            before.is_whitespace().then_some(path)
        }
    }
}

/// `text` with the mention being typed replaced by `path`, and a space after
/// it so the next word starts a word.
pub(crate) fn accepted(text: &str, path: &str) -> String {
    let Some(typed) = query(text) else {
        return text.to_owned();
    };
    let keep = text.len() - typed.len() - '@'.len_utf8();
    format!("{}@{path} ", &text[..keep])
}

/// The paths of `paths` that fuzzy-match `query`, best first, at most `limit`.
/// An empty query offers the list as it stands.
pub(crate) fn filtered(paths: &[String], query: &str, limit: usize) -> Vec<String> {
    let mut scored: Vec<(i32, &String)> = paths
        .iter()
        .filter_map(|path| {
            crate::interactive::tui::fuzzy_score(path, query).map(|score| (score, path))
        })
        .collect();
    // Stable on the path so two equally good matches keep one order: the walk's
    // own sort is the listing's, and a popup that reshuffled it would move the
    // row under the operator's finger.
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, path)| path.clone())
        .collect()
}

/// Every path of the run's workspace the file tools would list, sorted, bounded
/// by [`MAX_LISTED`] entries and [`LIST_BUDGET`].
pub(crate) fn candidates(workspace: &Workspace) -> Vec<String> {
    let query = SearchQuery {
        kind: SearchKind::Glob,
        pattern: "**/*".to_owned(),
        path: String::new(),
        include: None,
        regex: false,
        case_insensitive: false,
        entry: Some(SearchEntry::File),
        grep: Default::default(),
        private_prefixes: Vec::new(),
    };
    let limits = SearchLimits {
        entries: MAX_LISTED,
        matches: MATCH_CAP,
        timeout: LIST_BUDGET,
        ..SearchLimits::default()
    };
    let cancellation = saya_agent::CancellationToken::new();
    match workspace.search_controlled(&query, &limits, &cancellation) {
        Ok(outcome) => outcome
            .matches
            .into_iter()
            .map(|found| match found {
                SearchMatch::Path(path)
                | SearchMatch::Line { path, .. }
                | SearchMatch::Count { path, .. } => path,
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// `message` as the run receives it: the operator's words, then every file the
/// message mentions, read through the run's own contained workspace.
///
/// A message that mentions nothing is returned untouched — the request is
/// exactly the words that were typed — and so is one whose run has no
/// workspace: there is no file a run without one is allowed to read.
pub(crate) fn attach(message: &str, workspace: Option<&Workspace>) -> String {
    let paths = mentioned(message);
    if paths.is_empty() {
        return message.to_owned();
    }
    let mut carried = format!("{message}\n\n{HEADER}\n");
    let mut spent = 0usize;
    for path in paths {
        carried.push_str(&entry(&path, workspace, &mut spent));
    }
    carried
}

/// One mentioned file's block: its bytes, its size, or the refusal that kept
/// it out.
fn entry(path: &str, workspace: Option<&Workspace>, spent: &mut usize) -> String {
    let Some(workspace) = workspace else {
        return block(path, &format!("not attached: {}", NO_WORKSPACE));
    };
    if let Err(refusal) = admit(path) {
        return block(path, &format!("refused: {refusal}"));
    }
    let file = match workspace.read(path, MAX_FILE_BYTES) {
        Ok(file) => file,
        Err(error) => return block(path, &format!("refused: {}", error)),
    };
    if file.size > MAX_FILE_BYTES {
        return block(
            path,
            &format!(
                "not attached: {} bytes is over the {MAX_FILE_BYTES}-byte mention bound",
                file.size
            ),
        );
    }
    if *spent + file.bytes.len() > MAX_TOTAL_BYTES {
        return block(
            path,
            &format!("not attached: the {MAX_TOTAL_BYTES}-byte mention total is spent"),
        );
    }
    *spent += file.bytes.len();
    format!(
        "--- {path} ({} bytes) ---\n{}\n",
        file.size,
        String::from_utf8_lossy(&file.bytes)
    )
}

/// What a run with no workspace says about a mention: the read tool's own
/// refusal, so the model reads one sentence for one absence.
const NO_WORKSPACE: &str = "no workspace is available in this run";

/// One refused or unread block, in the header's own shape.
fn block(path: &str, reason: &str) -> String {
    format!("--- {path} ({reason}) ---\n")
}

/// The read tool's refusal for `path`, or `Ok` when it will read it: the one
/// privacy rule, stated in the read tool's own words.
fn admit(path: &str) -> Result<(), String> {
    if saya_harness::workspace::private_path(path) {
        return Err(private_sentence(path));
    }
    Ok(())
}

/// The sentence the read tool answers a private path with.
pub(crate) fn private_sentence(path: &str) -> String {
    saya_agent::ToolError::WorkspacePrivatePath {
        path: path.to_owned(),
    }
    .to_string()
}

#[cfg(test)]
#[path = "mentions_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "mentions_attach_tests.rs"]
mod attach_tests;
