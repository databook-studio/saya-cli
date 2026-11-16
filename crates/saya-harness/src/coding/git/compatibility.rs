//! Git version detection and the compatibility paths it selects.
//!
//! Saya supports Git down to the release shipped by Ubuntu 22.04 LTS. The
//! newer mechanisms it prefers are gated on the version that introduced them,
//! verified against the official release notes:
//!
//! - `--no-lazy-fetch` and `GIT_NO_LAZY_FETCH` arrived in Git 2.45.0
//!   (`RelNotes/2.45.0.txt`; `git.c` sets the variable for the option and
//!   `environment.c` reads it).
//! - `git worktree list --porcelain -z` arrived in Git 2.36.0
//!   (`RelNotes/2.36.0.txt`).

use std::fmt;

/// The oldest Git release Saya supports. Ubuntu 22.04 LTS ships 2.34.1, the
/// lowest version exercised by the compatibility tests.
pub const MINIMUM_SUPPORTED: GitVersion = GitVersion::new(2, 34, 0);

/// `--no-lazy-fetch` and `GIT_NO_LAZY_FETCH` both arrived in Git 2.45.0.
const NO_LAZY_FETCH_MIN: GitVersion = GitVersion::new(2, 45, 0);
/// NUL-terminated `git worktree list --porcelain` output arrived in 2.36.0.
const WORKTREE_LIST_Z_MIN: GitVersion = GitVersion::new(2, 36, 0);

/// A parsed Git release, ordered by major, minor, then patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

impl GitVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse the numeric prefix of `git version` output. Vendor suffixes such
    /// as `2.34.1.windows.1` and `2.39.5 (Apple Git-154)` keep their leading
    /// three components; a missing patch component reads as zero.
    pub fn parse(text: &str) -> Option<Self> {
        let token = text.split_ascii_whitespace().next()?;
        let mut parts = token.split('.');
        let major = numeric(parts.next()?)?;
        let minor = numeric(parts.next()?)?;
        let patch = parts.next().and_then(numeric).unwrap_or(0);
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for GitVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn numeric(part: &str) -> Option<u32> {
    let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// How Saya keeps Git from lazily fetching objects from a promisor remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LazyFetchProtection {
    /// Git 2.45 or newer: the global `--no-lazy-fetch` option.
    NoLazyFetchOption,
    /// Older Git: only partial-clone repositories are refused.
    RefusePartialClone,
}

impl LazyFetchProtection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoLazyFetchOption => "no_lazy_fetch_option",
            Self::RefusePartialClone => "refuse_partial_clone",
        }
    }
}

/// The version-dependent mechanisms available to this Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitCompatibility {
    version: GitVersion,
    version_known: bool,
    no_lazy_fetch_option: bool,
    worktree_list_zero_terminated: bool,
}

impl GitCompatibility {
    pub const fn minimum_supported_version() -> GitVersion {
        MINIMUM_SUPPORTED
    }

    pub fn from_version(version: GitVersion) -> Self {
        Self {
            version,
            version_known: true,
            no_lazy_fetch_option: version >= NO_LAZY_FETCH_MIN,
            worktree_list_zero_terminated: version >= WORKTREE_LIST_Z_MIN,
        }
    }

    /// An unparseable version is treated as the minimum supported release and
    /// every newer mechanism stays off.
    pub const fn assumed_minimum() -> Self {
        Self {
            version: MINIMUM_SUPPORTED,
            version_known: false,
            no_lazy_fetch_option: false,
            worktree_list_zero_terminated: false,
        }
    }

    pub const fn version(self) -> GitVersion {
        self.version
    }

    pub const fn version_known(self) -> bool {
        self.version_known
    }

    pub const fn no_lazy_fetch_option(self) -> bool {
        self.no_lazy_fetch_option
    }

    pub const fn worktree_list_zero_terminated(self) -> bool {
        self.worktree_list_zero_terminated
    }

    pub const fn lazy_fetch_protection(self) -> LazyFetchProtection {
        if self.no_lazy_fetch_option {
            LazyFetchProtection::NoLazyFetchOption
        } else {
            LazyFetchProtection::RefusePartialClone
        }
    }
}

/// The raw `git --version` line and what it selects.
#[derive(Debug, Clone)]
pub struct GitVersionInfo {
    raw: String,
    compatibility: GitCompatibility,
}

impl GitVersionInfo {
    /// Any output that is not a parseable `git version X.Y.Z` line is treated
    /// as the minimum supported version, never as a fatal error.
    pub(super) fn parse(line: String) -> Self {
        let compatibility = line
            .strip_prefix("git version ")
            .and_then(GitVersion::parse)
            .map_or_else(
                GitCompatibility::assumed_minimum,
                GitCompatibility::from_version,
            );
        Self {
            raw: line,
            compatibility,
        }
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub const fn compatibility(&self) -> GitCompatibility {
        self.compatibility
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> GitVersion {
        GitVersion::parse(text).expect("version parses")
    }

    #[test]
    fn parses_plain_and_vendor_version_strings() {
        assert_eq!(version("2.34.1"), GitVersion::new(2, 34, 1));
        assert_eq!(version("2.34.1.windows.1"), GitVersion::new(2, 34, 1));
        assert_eq!(version("2.39.5 (Apple Git-154)"), GitVersion::new(2, 39, 5));
        assert_eq!(version("2.45.0\n"), GitVersion::new(2, 45, 0));
        assert_eq!(version("2.34"), GitVersion::new(2, 34, 0));
    }

    #[test]
    fn rejects_strings_without_a_major_and_minor() {
        assert_eq!(GitVersion::parse("unknown"), None);
        assert_eq!(GitVersion::parse("2"), None);
        assert_eq!(GitVersion::parse(""), None);
    }

    #[test]
    fn selects_each_compatibility_path_at_its_release_cutoff() {
        let old = GitCompatibility::from_version(version("2.34.1"));
        assert!(!old.no_lazy_fetch_option());
        assert!(!old.worktree_list_zero_terminated());
        assert_eq!(
            old.lazy_fetch_protection(),
            LazyFetchProtection::RefusePartialClone
        );

        let listing = GitCompatibility::from_version(version("2.36.0"));
        assert!(!listing.no_lazy_fetch_option());
        assert!(listing.worktree_list_zero_terminated());
        assert_eq!(
            listing.lazy_fetch_protection(),
            LazyFetchProtection::RefusePartialClone
        );

        let before = GitCompatibility::from_version(version("2.44.9"));
        assert!(!before.no_lazy_fetch_option());
        assert!(before.worktree_list_zero_terminated());

        let current = GitCompatibility::from_version(version("2.45.0"));
        assert!(current.no_lazy_fetch_option());
        assert!(current.worktree_list_zero_terminated());
        assert_eq!(
            current.lazy_fetch_protection(),
            LazyFetchProtection::NoLazyFetchOption
        );

        let newer = GitCompatibility::from_version(version("2.50.1"));
        assert_eq!(
            newer.lazy_fetch_protection(),
            LazyFetchProtection::NoLazyFetchOption
        );
    }

    #[test]
    fn an_unknown_version_is_the_minimum_supported_and_disables_newer_paths() {
        let compatibility = GitCompatibility::assumed_minimum();
        assert_eq!(compatibility.version(), MINIMUM_SUPPORTED);
        assert!(!compatibility.version_known());
        assert!(!compatibility.no_lazy_fetch_option());
        assert!(!compatibility.worktree_list_zero_terminated());
        assert_eq!(
            compatibility.lazy_fetch_protection(),
            LazyFetchProtection::RefusePartialClone
        );
    }

    #[test]
    fn unparseable_version_output_is_the_minimum_supported() {
        let info = GitVersionInfo::parse("git version 2.34.1\n".to_owned());
        assert_eq!(
            info.compatibility(),
            GitCompatibility::from_version(GitVersion::new(2, 34, 1))
        );
        for line in ["git version unknown\n", "2.34.1\n", "\n"] {
            let info = GitVersionInfo::parse(line.to_owned());
            assert_eq!(info.raw(), line);
            assert_eq!(info.compatibility(), GitCompatibility::assumed_minimum());
        }
    }
}
