//! The one denial text a write outside the in-place `--write` scope carries:
//! the paths the job may write, bounded, and the two ways it can still finish.

use saya_harness::coding::integration::AssignedPaths;

/// Most scope entries one denial names; the rest are counted.
const MAX_NAMED_WRITE_PATHS: usize = 8;
/// Most bytes the named entries may take, so a set of long paths cannot make
/// the denial long. The first entry is always named, however long it is.
const MAX_NAMED_WRITE_PATH_BYTES: usize = 256;

/// The refusal a write outside the in-place `--write` scope carries. It names
/// what the job may write — bounded to `MAX_NAMED_WRITE_PATHS` entries and
/// `MAX_NAMED_WRITE_PATH_BYTES`, with the rest counted — and says the job can
/// still finish by writing inside them or reporting what it needs.
pub(crate) fn outside_write_scope(scope: &AssignedPaths) -> String {
    let entries = scope.entries();
    let mut shown = 0;
    let mut bytes = 0;
    for entry in entries {
        if shown == MAX_NAMED_WRITE_PATHS {
            break;
        }
        let next = bytes + entry.len() + usize::from(shown > 0) * 2;
        if shown > 0 && next > MAX_NAMED_WRITE_PATH_BYTES {
            break;
        }
        shown += 1;
        bytes = next;
    }
    let omitted = entries.len().saturating_sub(shown);
    let named = entries[..shown].join(", ");
    let mut text = if named.is_empty() {
        "the path is outside this job's write scope".to_owned()
    } else {
        format!("the path is outside this job's write scope; it may write {named}")
    };
    if omitted > 0 {
        text.push_str(&format!(" (and {omitted} more)"));
    }
    text.push_str(". Finish by writing inside those paths, or report what you need.");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(entries: &[&str]) -> AssignedPaths {
        AssignedPaths::parse(entries.iter().map(|entry| (*entry).to_owned())).unwrap()
    }

    #[test]
    fn the_denial_names_the_paths_the_job_may_write() {
        let text = outside_write_scope(&scope(&["src", "tests"]));
        assert!(text.contains("it may write src, tests"), "{text}");
        assert!(text.contains("report what you need"), "{text}");
        assert!(
            text.contains("Finish by writing inside those paths"),
            "{text}"
        );
    }

    #[test]
    fn the_denial_names_at_most_eight_paths_and_counts_the_rest() {
        let entries: Vec<String> = (0..10).map(|index| format!("p{index:02}")).collect();
        let text = outside_write_scope(&AssignedPaths::parse(entries.clone()).unwrap());
        for named in &entries[..8] {
            assert!(text.contains(named.as_str()), "missing {named}: {text}");
        }
        for omitted in &entries[8..] {
            assert!(!text.contains(omitted.as_str()), "named {omitted}: {text}");
        }
        assert!(text.contains("(and 2 more)"), "{text}");
    }

    #[test]
    fn the_denial_names_at_least_one_path_however_long_it_is() {
        let long = "d".repeat(400);
        let text = outside_write_scope(&AssignedPaths::parse([long.clone()]).unwrap());
        assert!(text.contains(&long), "{text}");
        assert!(!text.contains("and 0 more"), "{text}");
    }
}
