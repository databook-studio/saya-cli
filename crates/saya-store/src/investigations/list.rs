//! The bounded list scan over the document root (D2): at most the
//! collection cap + 1 candidate files, sorted by id, paginated at most
//! [`MAX_LIST_PAGE`] per page, with unreadable files reported as issues
//! instead of failing the page.

use saya_types::investigation::InvestigationId;

use crate::StoreError;

use super::{InvestigationRepository, LIST_SCAN_BOUND};

/// The largest page a caller may request.
pub const MAX_LIST_PAGE: usize = 50;

/// One readable investigation, as the bounded list scan reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationSummary {
    pub id: InvestigationId,
    pub revision: u32,
    pub name: String,
    pub dialect: saya_types::SqlDialect,
    pub connection: String,
    pub updated_unix_ms: i64,
}

/// One unreadable candidate file, reported instead of failing the page.
/// `error` is the `StoreError` variant name (e.g. `"Invalid"`), since store
/// errors are payload-free by contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationListIssue {
    pub file_stem: String,
    pub error: String,
}

/// One page of the bounded list scan: summaries and issues in id order,
/// plus how many candidates the scan saw and whether more exist beyond
/// this page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationPage {
    pub summaries: Vec<InvestigationSummary>,
    pub issues: Vec<InvestigationListIssue>,
    pub total_seen: usize,
    pub capped: bool,
}

impl InvestigationRepository {
    /// One page of saved-investigation summaries in id order. Candidates
    /// are the bounded scan's `<id>.json` files; only the page's slice is
    /// read and classified, so one bad file yields an
    /// [`InvestigationListIssue`] on its own page and never fails the scan.
    /// `capped` is true whenever more candidates may exist beyond this
    /// page — either the scan saw them or the scan hit its bound.
    pub fn list(&self, offset: usize, limit: usize) -> Result<InvestigationPage, StoreError> {
        if !(1..=MAX_LIST_PAGE).contains(&limit) {
            return Err(StoreError::invalid());
        }
        let candidates = self.scan_candidates()?;
        let total_seen = candidates.len();
        let start = offset.min(total_seen);
        let end = offset.saturating_add(limit).min(total_seen);
        let mut summaries = Vec::new();
        let mut issues = Vec::new();
        for id in &candidates[start..end] {
            match self.read_document(id) {
                Ok(Some(definition)) if definition.id == *id => {
                    summaries.push(InvestigationSummary {
                        id: id.clone(),
                        revision: definition.revision,
                        name: definition.name,
                        dialect: definition.dialect,
                        connection: definition.connection,
                        updated_unix_ms: definition.updated_unix_ms,
                    });
                }
                // Readable but internally inconsistent: the document's own
                // id disagrees with the file it was addressed by.
                Ok(Some(_)) => issues.push(InvestigationListIssue {
                    file_stem: id.as_str().to_owned(),
                    error: "Invalid".to_owned(),
                }),
                // The file vanished between the scan and this read; there
                // is nothing left to report.
                Ok(None) => {}
                Err(error) => issues.push(InvestigationListIssue {
                    file_stem: id.as_str().to_owned(),
                    error: format!("{error:?}"),
                }),
            }
        }
        let capped = total_seen == LIST_SCAN_BOUND || total_seen > offset.saturating_add(limit);
        Ok(InvestigationPage {
            summaries,
            issues,
            total_seen,
            capped,
        })
    }
}
