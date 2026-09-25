//! A minimal wildcard matcher for the workspace `glob`: `*` and `?` inside
//! one path segment, `**` as a whole segment spanning zero or more segments.
//! No dependency and no regex — and deliberately no backtracking matcher:
//! both the segment and the character match run as dynamic programming over
//! (pattern piece, text piece) pairs, so an adversarial pattern like
//! `a*a*a*a*b` against a long run of one character finishes in polynomial
//! time instead of hanging the turn. The matcher is pure string work and it
//! can only ever be handed paths the contained walk already resolved, so an
//! absolute or `..` pattern simply matches nothing — there is no route to an
//! escape through pattern text.

/// One atom of a single-segment pattern.
#[derive(Clone, Copy)]
enum Atom {
    /// One literal character.
    Literal(char),
    /// `?` — exactly one character.
    AnyChar,
    /// `*` — any run of characters, including none, within the component.
    AnyRun,
}

/// One pattern piece between `/` separators.
enum Segment {
    /// `**` — zero or more whole path segments.
    AnyDepth,
    /// Any other piece: wildcards matched within one path component.
    One(Vec<Atom>),
}

/// A compiled glob pattern: compile once, match many candidate paths.
pub(crate) struct Pattern {
    segments: Vec<Segment>,
}

impl Pattern {
    pub(crate) fn new(pattern: &str) -> Self {
        Self {
            segments: pattern
                .split('/')
                .map(|segment| {
                    if segment == "**" {
                        Segment::AnyDepth
                    } else {
                        Segment::One(
                            segment
                                .chars()
                                .map(|character| match character {
                                    '*' => Atom::AnyRun,
                                    '?' => Atom::AnyChar,
                                    other => Atom::Literal(other),
                                })
                                .collect(),
                        )
                    }
                })
                .collect(),
        }
    }

    /// Whether `path` — a `/`-separated path relative to the workspace root —
    /// matches this pattern.
    pub(crate) fn matches(&self, path: &str) -> bool {
        let components: Vec<&str> = path.split('/').collect();
        let texts: Vec<Vec<char>> = components
            .iter()
            .map(|component| component.chars().collect())
            .collect();
        let (n, m) = (self.segments.len(), components.len());
        // state[i][j]: segments[i..] match components[j..]. Filled from the
        // ends so each cell reads only cells that are already final.
        let mut state = vec![vec![false; m + 1]; n + 1];
        state[n][m] = true;
        for i in (0..n).rev() {
            for j in (0..=m).rev() {
                state[i][j] = match &self.segments[i] {
                    Segment::AnyDepth => state[i + 1][j] || (j < m && state[i][j + 1]),
                    Segment::One(atoms) => {
                        j < m && match_component(atoms, &texts[j]) && state[i + 1][j + 1]
                    }
                };
            }
        }
        state[0][0]
    }
}

/// Whether one pattern piece matches one path component, as dynamic
/// programming over (atom, character) pairs — never a backtracking walk.
fn match_component(atoms: &[Atom], text: &[char]) -> bool {
    let (n, m) = (atoms.len(), text.len());
    let mut state = vec![vec![false; m + 1]; n + 1];
    state[n][m] = true;
    for i in (0..n).rev() {
        for j in (0..=m).rev() {
            state[i][j] = match atoms[i] {
                Atom::AnyRun => state[i + 1][j] || (j < m && state[i][j + 1]),
                Atom::AnyChar => j < m && state[i + 1][j + 1],
                Atom::Literal(expected) => j < m && text[j] == expected && state[i + 1][j + 1],
            };
        }
    }
    state[0][0]
}

#[cfg(test)]
mod tests {
    use super::Pattern;

    fn matches(pattern: &str, path: &str) -> bool {
        Pattern::new(pattern).matches(path)
    }

    #[test]
    fn star_and_question_match_within_one_segment_only() {
        assert!(matches("*", "notes"));
        assert!(matches("*.md", "readme.md"));
        assert!(
            !matches("*.md", "notes/readme.md"),
            "`*` must not cross `/`"
        );
        assert!(matches("n?tes", "notes"));
        assert!(!matches("n?tes", "ntes"), "`?` is exactly one character");
        assert!(!matches("n?tes", "notees"));
    }

    #[test]
    fn double_star_spans_segments() {
        assert!(matches("**", "any/thing/at/all"));
        assert!(matches("**/*.md", "notes/deep/readme.md"));
        assert!(
            matches("**/*.md", "readme.md"),
            "`**` also matches zero segments"
        );
        assert!(matches("notes/**/*.md", "notes/deep/readme.md"));
        assert!(matches("notes/**/readme.md", "notes/readme.md"));
        assert!(!matches("notes/*.md", "notes/deep/readme.md"));
    }

    #[test]
    fn matching_is_literal_and_case_sensitive() {
        assert!(matches("a*b", "a b"), "there is no class syntax to exclude");
        assert!(matches("READ*", "README.md"));
        assert!(!matches("read*", "README.md"));
        assert!(!matches("a.b", "axb"), "there is no `.` wildcard");
    }

    /// The adversarial case for wildcard matching: a chain of `a*` pieces
    /// against a long run of `a` with no `b` anywhere. A backtracking matcher
    /// goes exponential here; these calls must simply return, promptly, so an
    /// exponential matcher cannot ship unnoticed.
    #[test]
    fn an_adversarial_star_chain_terminates() {
        let run = "a".repeat(100_000);
        assert!(!matches("a*a*a*a*b", &run));
        assert!(matches("a*a*a*a*b", &format!("{run}b")));
    }
}
