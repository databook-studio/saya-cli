//! The bounded commit list of one managed checkout: the commits reachable
//! from its `HEAD` and not from the base commit it was created at. The count
//! is bounded by the caller and the page says whether more exist; each subject
//! is bounded too, because a commit message is model-controlled text.

use std::path::Path;

use saya_agent::CancellationToken;
use saya_types::{GitObjectId, ManagedCheckoutPath};

use super::{
    command::{GitError, GitOperation, require_success},
    request::GitRequest,
    service::GitService,
};

/// Most bytes one commit subject may occupy in a page.
const MAX_SUBJECT_BYTES: usize = 200;

/// One commit in a page: its object id and the bounded subject line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommit {
    pub sha: String,
    pub subject: String,
    pub subject_truncated: bool,
}

/// One bounded page of commits. `truncated` is true when the checkout has more
/// commits since the base than the requested maximum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitPage {
    pub commits: Vec<GitCommit>,
    pub truncated: bool,
}

impl GitService {
    /// The commits in `checkout` since `base`, newest first, at most `maximum`
    /// of them. A page that is full is reported truncated even when the next
    /// commit is the only one beyond it.
    pub async fn commits_since(
        &self,
        checkout: &ManagedCheckoutPath,
        base: &GitObjectId,
        maximum: usize,
        cancellation: &CancellationToken,
    ) -> Result<CommitPage, GitError> {
        let _guard = self.operation_guard().await;
        self.ensure_supported(cancellation).await?;
        let path = Path::new(checkout.as_str());
        if !path.starts_with(&self.environment.worktrees) {
            return Err(GitError::InvalidManagedPath);
        }
        let wanted = maximum
            .checked_add(1)
            .ok_or(GitError::OutputLimitExceeded)?;
        let output = self
            .run_request(
                path,
                GitRequest::LogSince {
                    base,
                    limit: wanted,
                },
                cancellation,
            )
            .await?;
        let output = require_success(output, GitOperation::Read)?;
        parse(&output.stdout, maximum)
    }
}

fn parse(stdout: &[u8], maximum: usize) -> Result<CommitPage, GitError> {
    let text = String::from_utf8_lossy(stdout);
    let mut commits = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let (sha, subject) = line.split_once('\0').ok_or(GitError::MalformedOutput)?;
        if sha.is_empty() || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(GitError::MalformedOutput);
        }
        let subject = subject.strip_suffix('\r').unwrap_or(subject);
        let (subject, subject_truncated) = bounded_subject(subject);
        commits.push(GitCommit {
            sha: sha.to_owned(),
            subject,
            subject_truncated,
        });
    }
    let truncated = commits.len() > maximum;
    commits.truncate(maximum);
    Ok(CommitPage { commits, truncated })
}

fn bounded_subject(subject: &str) -> (String, bool) {
    if subject.len() <= MAX_SUBJECT_BYTES {
        return (subject.to_owned(), false);
    }
    let mut end = MAX_SUBJECT_BYTES;
    while !subject.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}…", &subject[..end]), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_reports_the_commit_beyond_the_maximum_as_truncated() {
        let stdout = b"a\0one\nb\0two\n";
        let page = parse(stdout, 1).expect("the page parses");
        assert_eq!(page.commits.len(), 1);
        assert_eq!(page.commits[0].sha, "a");
        assert!(page.truncated);
    }

    #[test]
    fn a_line_without_a_separator_is_malformed() {
        assert!(matches!(
            parse(b"nonsense\n", 4),
            Err(GitError::MalformedOutput)
        ));
    }

    #[test]
    fn a_long_subject_is_bounded_and_marked() {
        let (subject, truncated) = bounded_subject(&"x".repeat(MAX_SUBJECT_BYTES + 10));
        assert!(truncated);
        assert!(subject.ends_with('…'));
    }
}
