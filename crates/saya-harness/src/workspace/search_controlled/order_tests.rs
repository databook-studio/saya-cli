//! The ordering itself, including the case a real filesystem cannot be made to
//! produce on demand: an entry whose stat named no usable modification time.
use super::*;

fn hit(path: &str, seconds: Option<u64>) -> GlobHit {
    GlobHit {
        path: path.into(),
        modified: seconds
            .map(|seconds| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
    }
}

fn served(hits: Vec<GlobHit>, cap: usize) -> Vec<String> {
    let (matches, truncated) = serve(hits, cap);
    assert!(
        !truncated,
        "these orderings fit the cap they were served under"
    );
    matches
        .iter()
        .map(|hit| match hit {
            SearchMatch::Path(path) => path.clone(),
            SearchMatch::Line { .. } | SearchMatch::Count { .. } => {
                panic!("a glob serves paths, not lines or counts")
            }
        })
        .collect()
}

/// A filesystem that refuses mtime, or a stat whose time the clock cannot
/// name, must not fail the call: those entries fall back to path order and
/// sort after every entry that does have a time.
#[test]
fn an_unusable_mtime_falls_back_to_path_order() {
    assert_eq!(
        served(
            vec![
                hit("b.txt", None),
                hit("a.txt", None),
                hit("z.txt", Some(5))
            ],
            500
        ),
        ["z.txt", "a.txt", "b.txt"]
    );
    assert_eq!(
        served(vec![hit("b.txt", None), hit("a.txt", None)], 500),
        ["a.txt", "b.txt"],
        "with no usable time at all, the whole answer is path order"
    );
}

#[test]
fn equal_times_break_by_path_and_the_cap_drops_the_oldest() {
    assert_eq!(
        served(
            vec![
                hit("b.txt", Some(7)),
                hit("a.txt", Some(7)),
                hit("c.txt", Some(9))
            ],
            500
        ),
        ["c.txt", "a.txt", "b.txt"]
    );
    let (matches, truncated) = serve(vec![hit("old.txt", Some(1)), hit("new.txt", Some(2))], 1);
    assert!(truncated);
    assert_eq!(matches.len(), 1);
    assert!(matches!(matches[0], SearchMatch::Path(ref path) if path == "new.txt"));
}
