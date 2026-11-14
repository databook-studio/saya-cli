use super::super::*;

#[tokio::test]
async fn scoped_nested_ignore_precedence_and_translation_match_git() {
    let fixture = Fixture::new("oracle");
    fixture.put(".gitignore", b"ignored/\n!ignored/kept.rs\n*.tmp\n!important.tmp\n/root-only.rs\nlogs/**/noise?.txt\nbuild/*\n!build/keep/\n!build/keep/*.rs\ndeny.*\n!deny.keep\ndeny.keep/\nrange[0-9].txt\nclass[.-0].txt\nliteral\\[bracket\\].txt\nliteral{one,two}.txt\n[[:digit:]].num\n\\#hash.txt\n\\!bang.txt\nspace\\ \n");
    fixture.put("src/.gitignore", b"*.local\n!important.local\n");
    let candidates = [
        "ignored/kept.rs",
        "drop.tmp",
        "important.tmp",
        "root-only.rs",
        "src/root-only.rs",
        "logs/noise1.txt",
        "logs/deep/noise2.txt",
        "build/drop.rs",
        "build/keep/ok.rs",
        "deny.keep",
        "range5.txt",
        "rangeA.txt",
        "class0.txt",
        "class/.txt",
        "literal[bracket].txt",
        "literal{one,two}.txt",
        "literalone.txt",
        "literaltwo.txt",
        "5.num",
        "A.num",
        "#hash.txt",
        "!bang.txt",
        "space ",
        "src/drop.local",
        "src/important.local",
        "src/public.rs",
    ];
    for path in candidates {
        fixture.put(path, b"needle\n");
    }
    let ignored = git_ignored(&fixture, &candidates).await;
    for scope in ["", "src", "logs/deep", "build/keep", "ignored"] {
        let result = fixture.search(
            &query(SearchKind::Glob, "**", scope),
            SearchLimits::default(),
        );
        assert!(
            result.coverage_complete,
            "ordinary supported rules need exact coverage: {result:?}"
        );
        let found = paths(&result);
        for path in candidates
            .iter()
            .filter(|path| scope.is_empty() || path.starts_with(&format!("{scope}/")))
        {
            assert_eq!(
                found.contains(path),
                !ignored.contains(*path),
                "Git parity for {path} in {scope}"
            );
        }
    }
    let mut grep = query(SearchKind::Grep, "^needle$", "src");
    grep.regex = true;
    grep.include = Some("*.rs".into());
    let result = fixture.search(&grep, SearchLimits::default());
    assert_eq!(paths(&result), ["src/public.rs", "src/root-only.rs"]);
    assert_eq!(result.total_matches, Some(2));
    assert!(result.coverage_complete);
}

#[tokio::test]
async fn ascii_posix_classes_and_utf8_or_foreign_boms_match_git() {
    let classes = [
        ("alnum", "A", "!"),
        ("alpha", "z", "7"),
        ("blank", " ", "x"),
        ("cntrl", "\u{1}", "A"),
        ("digit", "7", "q"),
        ("graph", "!", " "),
        ("lower", "q", "Q"),
        ("print", " ", "\u{1}"),
        ("punct", "!", "A"),
        ("space", "\t", "x"),
        ("upper", "Q", "q"),
        ("xdigit", "f", "g"),
    ];
    for (class, yes, no) in classes {
        let fixture = Fixture::new(class);
        fixture.put(
            ".gitignore",
            format!("[[:{class}:]].*-sample\n[![:{class}:]].*-other\n").as_bytes(),
        );
        let yes = format!("{yes}.yes-sample");
        let no = format!("{no}.no-sample");
        fixture.put(&yes, b"content");
        fixture.put(&no, b"content");
        let other_yes = yes.replace("-sample", "-other");
        let other_no = no.replace("-sample", "-other");
        fixture.put(&other_yes, b"content");
        fixture.put(&other_no, b"content");
        let ignored = git_ignored(&fixture, &[&yes, &no, &other_yes, &other_no]).await;
        let result = fixture.search(&query(SearchKind::Glob, "**", ""), SearchLimits::default());
        assert!(result.coverage_complete, "class {class}: {result:?}");
        assert_eq!(
            paths(&result).contains(&yes.as_str()),
            !ignored.contains(&yes),
            "{class} positive"
        );
        assert_eq!(
            paths(&result).contains(&no.as_str()),
            !ignored.contains(&no),
            "{class} negative"
        );
        for path in [&other_yes, &other_no] {
            assert_eq!(
                paths(&result).contains(&path.as_str()),
                !ignored.contains(path),
                "{class} negated {path}"
            );
        }
    }
    for (label, rules, candidates) in [
        (
            "utf8-bom",
            b"\xef\xbb\xbfbom.txt\n".as_slice(),
            vec!["bom.txt", "other.txt"],
        ),
        (
            "foreign-bom",
            b"+/v8-bom.txt\n".as_slice(),
            vec!["+/v8-bom.txt", "-bom.txt"],
        ),
    ] {
        let fixture = Fixture::new(label);
        fixture.put(".gitignore", rules);
        for path in &candidates {
            fixture.put(path, b"content");
        }
        let ignored = git_ignored(&fixture, &candidates).await;
        let result = fixture.search(&query(SearchKind::Glob, "**", ""), SearchLimits::default());
        assert!(result.coverage_complete, "{label}: {result:?}");
        for path in candidates {
            assert_eq!(
                paths(&result).contains(&path),
                !ignored.contains(path),
                "{label} {path}"
            );
        }
    }
}

#[tokio::test]
async fn ignore_double_star_prefix_cannot_expose_newline_ancestor_matches() {
    let fixture = Fixture::new("newline-ignore-oracle");
    fixture.put(".gitignore", b"hidden.txt\n");
    let hidden = "dir\nname/hidden.txt";
    fixture.put(hidden, b"needle\n");
    fixture.put("visible.txt", b"needle\n");
    let ignored = git_ignored(&fixture, &[hidden, "visible.txt"]).await;
    assert!(
        ignored.contains(hidden),
        "real Git must ignore the actual newline-parent path"
    );
    assert!(
        !ignored.contains("visible.txt"),
        "independent oracle positive control"
    );
    let result = fixture.search(&query(SearchKind::Glob, "**", ""), SearchLimits::default());
    assert!(result.coverage_complete);
    assert!(
        !paths(&result).contains(&hidden),
        "an admitted Git-ignored path must never be exposed by regex translation"
    );
    assert!(paths(&result).contains(&"visible.txt"));
}
