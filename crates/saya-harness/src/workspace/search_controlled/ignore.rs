use super::{
    ignore_translation::translate,
    io::{FileRead, ReadBudget},
    types::{Control, SearchError, SearchStop},
};
use crate::workspace::search_directory::Directory;
use gix_ignore::glob::pattern::Mode;
use regex::bytes::{Regex, RegexBuilder, RegexSet, RegexSetBuilder};

struct Rule {
    negative: bool,
    directory_only: bool,
}
struct Frame {
    base: String,
    matcher: Option<RegexSet>,
    rules: Vec<Rule>,
}

pub(super) struct IgnoreStack {
    frames: Vec<Frame>,
    raw_bytes: usize,
    rules: usize,
    regex_bytes: usize,
    active_sets: usize,
}
impl IgnoreStack {
    pub fn new(initial_sets: usize) -> Self {
        Self {
            frames: Vec::new(),
            raw_bytes: 0,
            rules: 0,
            regex_bytes: 0,
            active_sets: initial_sets,
        }
    }
    pub fn push(
        &mut self,
        base: &str,
        directory: &Directory,
        reads: &mut ReadBudget,
        control: &Control<'_>,
    ) -> Result<Result<(), SearchStop>, SearchError> {
        let path = if base.is_empty() {
            ".gitignore".into()
        } else {
            format!("{base}/.gitignore")
        };
        let bytes = match reads.read(directory, &path, 64 * 1024, control)? {
            Ok(FileRead::Bytes(bytes)) => bytes,
            Ok(FileRead::Missing) => {
                self.frames.push(Frame {
                    base: base.into(),
                    matcher: None,
                    rules: Vec::new(),
                });
                return Ok(Ok(()));
            }
            Ok(FileRead::Unavailable) => return Ok(Err(SearchStop::IgnorePolicyUnavailable)),
            Err(stop) => return Ok(Err(stop)),
        };
        self.raw_bytes = self.raw_bytes.saturating_add(bytes.len());
        if self.raw_bytes > 256 * 1024
            || std::str::from_utf8(&bytes).is_err()
            || bytes.contains(&0)
            || bytes
                .split(|byte| *byte == b'\n')
                .any(|line| line.len() > 1024)
        {
            return Ok(Err(SearchStop::IgnorePolicyUnavailable));
        }
        let raw = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let mut protected = b"# bounded workspace ignore\n".to_vec();
        protected.extend_from_slice(raw);
        let mut patterns = Vec::new();
        let mut rules = Vec::new();
        for (pattern, _, _) in gix_ignore::parse(&protected, false) {
            if !control.check()? {
                return Ok(Err(SearchStop::Deadline));
            }
            self.rules += 1;
            if self.rules > 4096 {
                return Ok(Err(SearchStop::IgnorePolicyUnavailable));
            }
            let text = match std::str::from_utf8(&pattern.text) {
                Ok(text) => text,
                Err(_) => return Ok(Err(SearchStop::IgnorePolicyUnavailable)),
            };
            let basename =
                pattern.mode.contains(Mode::NO_SUB_DIR) && !pattern.mode.contains(Mode::ABSOLUTE);
            let regex = match translate(text, basename) {
                Ok(regex) => regex,
                Err(()) => return Ok(Err(SearchStop::IgnorePolicyUnavailable)),
            };
            self.regex_bytes = self.regex_bytes.saturating_add(regex.len());
            if regex.len() > 8192 || self.regex_bytes > 1024 * 1024 {
                return Ok(Err(SearchStop::IgnorePolicyUnavailable));
            }
            patterns.push(regex);
            rules.push(Rule {
                negative: pattern.mode.contains(Mode::NEGATIVE),
                directory_only: pattern.mode.contains(Mode::MUST_BE_DIR),
            });
        }
        if !control.check()? {
            return Ok(Err(SearchStop::Deadline));
        }
        let matcher = if patterns.is_empty() {
            None
        } else {
            if self.active_sets >= 16 {
                return Ok(Err(SearchStop::IgnorePolicyUnavailable));
            }
            match bounded_set(&patterns, false) {
                Ok(set) => Some(set),
                Err(()) => return Ok(Err(SearchStop::IgnorePolicyUnavailable)),
            }
        };
        if !control.check()? {
            return Ok(Err(SearchStop::Deadline));
        }
        if matcher.is_some() {
            self.active_sets += 1;
        }
        self.frames.push(Frame {
            base: base.into(),
            matcher,
            rules,
        });
        Ok(Ok(()))
    }
    pub fn pop(&mut self) {
        if self
            .frames
            .pop()
            .is_some_and(|frame| frame.matcher.is_some())
        {
            self.active_sets -= 1;
        }
    }
    pub fn ignored(
        &self,
        path: &str,
        directory: bool,
        control: &Control<'_>,
    ) -> Result<Result<bool, SearchStop>, SearchError> {
        let mut ignored = false;
        for frame in &self.frames {
            let relative = if frame.base.is_empty() {
                Some(path)
            } else {
                path.strip_prefix(&frame.base)
                    .and_then(|tail| tail.strip_prefix('/'))
            };
            if let (Some(relative), Some(matcher)) = (relative, &frame.matcher) {
                if !control.check()? {
                    return Ok(Err(SearchStop::Deadline));
                }
                let matches = matcher.matches(relative.as_bytes());
                if !control.check()? {
                    return Ok(Err(SearchStop::Deadline));
                }
                for index in matches.iter() {
                    let rule = &frame.rules[index];
                    if !rule.directory_only || directory {
                        ignored = !rule.negative;
                    }
                }
            }
        }
        Ok(Ok(ignored))
    }
}

pub(super) fn bounded_set(patterns: &[String], case_insensitive: bool) -> Result<RegexSet, ()> {
    RegexSetBuilder::new(patterns)
        .size_limit(1024 * 1024)
        .dfa_size_limit(1024 * 1024)
        .nest_limit(64)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|_| ())
}

/// One pattern under the same compilation bounds [`bounded_set`] states, for
/// the searches that need to know *where* a match is: a multiline match's
/// start line, and a per-file hit count. A pattern the bounds refuse is a
/// typed refusal, never a silently empty search.
pub(super) fn bounded_regex(pattern: &str, case_insensitive: bool) -> Result<Regex, ()> {
    RegexBuilder::new(pattern)
        .size_limit(1024 * 1024)
        .dfa_size_limit(1024 * 1024)
        .nest_limit(64)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|_| ())
}
