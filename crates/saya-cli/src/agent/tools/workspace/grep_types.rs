//! The bounded `type` map a `grep` may state: a ripgrep-style language name
//! and the file extensions it admits. The map is fixed and small on purpose —
//! it is the whole vocabulary the tool accepts, so a name outside it is a
//! typed refusal naming what is supported, never a silently empty search.

/// One language: its name, and the lowercase extensions (no dot) it admits.
pub(in crate::agent::tools) struct GrepType {
    pub(in crate::agent::tools) name: &'static str,
    pub(in crate::agent::tools) extensions: &'static [&'static str],
}

pub(in crate::agent::tools) const GREP_TYPES: &[GrepType] = &[
    GrepType {
        name: "c",
        extensions: &["c", "h"],
    },
    GrepType {
        name: "cpp",
        extensions: &["cc", "cpp", "cxx", "h", "hh", "hpp"],
    },
    GrepType {
        name: "cs",
        extensions: &["cs"],
    },
    GrepType {
        name: "css",
        extensions: &["css"],
    },
    GrepType {
        name: "go",
        extensions: &["go"],
    },
    GrepType {
        name: "html",
        extensions: &["htm", "html"],
    },
    GrepType {
        name: "java",
        extensions: &["java"],
    },
    GrepType {
        name: "js",
        extensions: &["cjs", "js", "jsx", "mjs"],
    },
    GrepType {
        name: "json",
        extensions: &["json"],
    },
    GrepType {
        name: "md",
        extensions: &["markdown", "md"],
    },
    GrepType {
        name: "php",
        extensions: &["php"],
    },
    GrepType {
        name: "py",
        extensions: &["py", "pyi"],
    },
    GrepType {
        name: "rb",
        extensions: &["rb"],
    },
    GrepType {
        name: "rust",
        extensions: &["rs"],
    },
    GrepType {
        name: "sh",
        extensions: &["bash", "sh", "zsh"],
    },
    GrepType {
        name: "sql",
        extensions: &["sql"],
    },
    GrepType {
        name: "toml",
        extensions: &["toml"],
    },
    GrepType {
        name: "ts",
        extensions: &["cts", "mts", "ts", "tsx"],
    },
    GrepType {
        name: "txt",
        extensions: &["txt"],
    },
    GrepType {
        name: "xml",
        extensions: &["xml"],
    },
    GrepType {
        name: "yaml",
        extensions: &["yaml", "yml"],
    },
];

/// The names the tool accepts as one line, for the refusal message. A test
/// holds it to
/// [`GREP_TYPES`].
pub(in crate::agent::tools) const NAMES: &str = "c, cpp, cs, css, go, html, java, js, json, md, php, py, rb, rust, sh, sql, toml, ts, txt, \
     xml, yaml";

pub(in crate::agent::tools) fn lookup(name: &str) -> Option<&'static GrepType> {
    GREP_TYPES.iter().find(|entry| entry.name == name)
}

/// Whether `pattern` contains a character a regex engine reads as syntax.
/// Only used to explain a zero-match literal search: the default stays
/// literal, and a pattern without these characters gets no note.
pub(in crate::agent::tools) fn has_regex_metacharacters(pattern: &str) -> bool {
    pattern.bytes().any(|byte| {
        matches!(
            byte,
            b'\\'
                | b'.'
                | b'+'
                | b'*'
                | b'?'
                | b'('
                | b')'
                | b'|'
                | b'['
                | b']'
                | b'{'
                | b'}'
                | b'^'
                | b'$'
        )
    })
}

/// The context-line bound the harness enforces. The schema states the same
/// number and validation refuses anything above it, so no `context` can ask
/// for more work than the search's own ceiling allows.
pub(in crate::agent::tools) const MAX_CONTEXT: usize = saya_harness::workspace::MAX_CONTEXT;

/// The largest `head_limit` a caller may state: the harness's own match
/// ceiling, so a limit narrows a search and can never raise its bound.
pub(in crate::agent::tools) fn head_limit_ceiling() -> usize {
    saya_harness::workspace::SearchLimits::default().matches
}

#[cfg(test)]
mod tests {
    use super::{GREP_TYPES, NAMES, head_limit_ceiling};

    /// The refusal message's hand-written line has to match the map exactly.
    #[test]
    fn names_line_matches_the_map() {
        let names: Vec<&str> = GREP_TYPES.iter().map(|entry| entry.name).collect();
        assert_eq!(names.join(", "), NAMES);
    }

    /// A `type` is only useful if it selects something; an empty extension
    /// list would silently match every file.
    #[test]
    fn every_type_admits_an_extension() {
        for entry in GREP_TYPES {
            assert!(
                !entry.extensions.is_empty(),
                "{} has no extensions",
                entry.name
            );
        }
    }

    #[test]
    fn head_limit_ceiling_is_the_search_match_bound() {
        assert_eq!(head_limit_ceiling(), 500);
    }
}
