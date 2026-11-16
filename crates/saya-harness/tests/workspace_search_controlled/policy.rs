use super::super::*;
use saya_harness::workspace::SearchStop;

#[test]
fn unavailable_ignore_policy_and_active_set_limits_are_honest() {
    for (label, bytes) in [
        ("invalid-utf8", vec![0xff]),
        ("nul", b"pattern\0\n".to_vec()),
        ("too-large", b"x\n".repeat(32_769)),
        ("long-rule", vec![b'x'; 1025]),
        ("too-many", b"p\n".repeat(4097)),
        ("mixed-posix", b"[a[:digit:]].rs\n".to_vec()),
    ] {
        let fixture = Fixture::new(label);
        fixture.put(".gitignore", &bytes);
        fixture.put("target.rs", b"needle");
        let result = fixture.search(
            &query(SearchKind::Glob, "**/*.rs", ""),
            SearchLimits::default(),
        );
        assert!(
            !result.coverage_complete && result.truncated,
            "{label}: {result:?}"
        );
        assert_eq!(result.total_matches, None);
        assert_eq!(
            result.stop_reason,
            Some(SearchStop::IgnorePolicyUnavailable)
        );
        assert!(result.matches.is_empty());
    }
    let fixture = Fixture::new("active-sets");
    let mut path = String::new();
    for depth in 0..17 {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(&format!("d{depth:02}"));
        fixture.put(&format!("{path}/.gitignore"), b"unmatched\n");
    }
    fixture.put(&format!("{path}/target.hit"), b"needle");
    let result = fixture.search(
        &query(SearchKind::Glob, "**/*.hit", ""),
        SearchLimits::default(),
    );
    assert_eq!(
        result.stop_reason,
        Some(SearchStop::IgnorePolicyUnavailable)
    );
    assert!(!result.coverage_complete && result.total_matches.is_none());
}

#[test]
fn scoped_ancestor_work_respects_lower_and_hard_depth_limits() {
    let fixture = Fixture::new("ancestor-depth");
    let mut results = Vec::new();
    for (limit, components) in [(2, 3), (64, 65)] {
        let path = std::iter::repeat_n("d", components)
            .collect::<Vec<_>>()
            .join("/");
        fs::create_dir_all(fixture.workspace.root().join(&path)).unwrap();
        results.push(fixture.search(
            &query(SearchKind::Glob, "**", &path),
            SearchLimits {
                depth: limit,
                ..SearchLimits::default()
            },
        ));
    }
    for result in results {
        assert!(
            !result.coverage_complete && result.truncated,
            "scoped ancestry must not evade depth: {result:?}"
        );
        assert_eq!(result.total_matches, None);
        assert_eq!(result.stop_reason, Some(SearchStop::Depth));
    }
}

#[cfg(unix)]
#[test]
fn same_byte_private_content_and_links_cannot_be_resurrected() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new("privacy");
    fixture.put(
        ".gitignore",
        b"!.env\n!.codex/\n!ordinary.rs\n!private-zone/\n",
    );
    for path in [".env", ".codex/context.md", "private-zone/notes.txt"] {
        fixture.put(path, b"needle");
    }
    fixture.put("ordinary.rs", b"password=synthetic-search-private needle\n");
    fixture.put("public.rs", b"needle\n");
    fs::write(fixture.outer.join("outside.rs"), b"outside needle").unwrap();
    symlink(
        fixture.outer.join("outside.rs"),
        fixture.workspace.root().join("decoy.rs"),
    )
    .unwrap();
    let mut grep = query(SearchKind::Grep, "needle", "");
    grep.private_prefixes.push("private-zone".into());
    let result = fixture.search(&grep, SearchLimits::default());
    assert_eq!(paths(&result), ["public.rs"]);
    let wire = serde_json::to_string(&result).unwrap();
    for marker in [
        "synthetic-search-private",
        ".env",
        ".codex",
        "private-zone",
        "outside needle",
        "decoy",
    ] {
        assert!(!wire.contains(marker));
    }
    fs::remove_file(fixture.workspace.root().join(".gitignore")).unwrap();
    symlink(
        fixture.outer.join("outside.rs"),
        fixture.workspace.root().join(".gitignore"),
    )
    .unwrap();
    let result = fixture.search(&grep, SearchLimits::default());
    assert_eq!(
        result.stop_reason,
        Some(SearchStop::IgnorePolicyUnavailable)
    );
    assert!(!result.coverage_complete && result.total_matches.is_none());
    assert!(result.matches.is_empty());
}
