//! Public tools must return useful bounded search data past a 2k-file walk.
use super::Sandbox;
use saya_agent::ToolExecutor;
use serde_json::{Value, json};
use std::fs;

const FILES: usize = 2_100;
const MATCH_CAP: usize = 500;
const JSON_CAP: usize = 48 * 1024;

#[tokio::test]
async fn large_sibling_glob_returns_bounded_partial_data() {
    sibling_search("glob").await;
}

#[tokio::test]
async fn large_sibling_grep_returns_bounded_partial_data() {
    sibling_search("grep").await;
}

async fn sibling_search(tool: &str) {
    let sandbox = Sandbox::new(&format!("large-sibling-{tool}"));
    sandbox.plant(
        (0..FILES).map(|index| format!("f{index:04}.txt")),
        b"needle\n",
    );
    let arguments = if tool == "glob" {
        json!({"pattern":"**/*.txt"})
    } else {
        json!({"pattern":"needle"})
    };
    let result = sandbox.tools().execute(tool, arguments).await;
    assert!(
        result.is_ok(),
        "{tool} must return bounded search data for {FILES} real sibling files; \
         the old directory-enumeration refusal is not useful: {result:?}"
    );
    let result = result.unwrap();
    let matches = bounded_matches(&result);
    assert!(!matches.is_empty(), "a capped search must retain real hits");
    assert_eq!(
        result["truncated"], true,
        "2100 hits exceed the retained cap"
    );
    let found = result["matches_found"]
        .as_u64()
        .expect("observed lower bound");
    assert!(found >= matches.len() as u64 && found <= FILES as u64);
    match result["coverage_complete"].as_bool() {
        Some(true) => assert_eq!(result["total_matches"], FILES),
        Some(false) => assert!(
            result["total_matches"].is_null(),
            "a cutoff has no exact total"
        ),
        None => panic!("bounded results must state whether coverage completed"),
    }
    for hit in matches {
        if tool == "glob" {
            assert!(
                hit.as_str()
                    .is_some_and(|path| path.starts_with('f') && path.ends_with(".txt"))
            );
        } else {
            assert_eq!(hit["line"], 1);
            assert_eq!(hit["text"], "needle");
            assert!(
                hit.get("truncated").is_none(),
                "a hit whose text fits carries no `truncated`: {hit}"
            );
        }
    }
}

#[tokio::test]
async fn large_distributed_glob_completes_a_selective_search() {
    distributed_search("glob").await;
}

#[tokio::test]
async fn large_distributed_grep_completes_a_selective_search() {
    distributed_search("grep").await;
}

async fn distributed_search(tool: &str) {
    let sandbox = Sandbox::new(&format!("large-distributed-{tool}"));
    // No directory exceeds 101 files, and only one path/line matches, so the
    // baseline refusal comes from cumulative traversal, not per-dir/match caps.
    for group in 0..21 {
        let directory = sandbox.ws.root().join(format!("group-{group:02}"));
        fs::create_dir(&directory).expect("fixture directory must create");
        for file in 0..100 {
            fs::write(directory.join(format!("f{file:03}.txt")), b"plain text\n")
                .expect("fixture file must create");
        }
    }
    let path = "group-00/needle.hit";
    fs::write(sandbox.ws.root().join(path), b"needle\n").expect("real hit must create");
    let arguments = if tool == "glob" {
        json!({"pattern":"**/*.hit"})
    } else {
        json!({"pattern":"needle"})
    };
    let result = sandbox.tools().execute(tool, arguments).await;
    assert!(
        result.is_ok(),
        "{tool} must search a small distributed repository beyond 2000 entries \
         without the old cumulative refusal: {result:?}"
    );
    let result = result.unwrap();
    let matches = bounded_matches(&result);
    assert_eq!(matches.len(), 1, "the actual selective corpus has one hit");
    if tool == "glob" {
        assert_eq!(matches[0], path);
    } else {
        assert_eq!(matches[0]["path"], path);
        assert_eq!(matches[0]["line"], 1);
        assert_eq!(matches[0]["text"], "needle");
        assert_eq!(result["files_scanned"], FILES + 1);
        assert_eq!(result["files_skipped"], 0);
    }
    assert_eq!(result["coverage_complete"], true);
    assert_eq!(result["truncated"], false);
    assert_eq!(result["matches_found"], 1);
    assert_eq!(result["total_matches"], 1);
}

fn bounded_matches(result: &Value) -> &[Value] {
    assert!(
        serde_json::to_vec(result).unwrap().len() <= JSON_CAP,
        "the complete escaped JSON result must fit its bound"
    );
    let matches = result["matches"].as_array().expect("valid matches array");
    assert!(matches.len() <= MATCH_CAP, "returned hit count is bounded");
    matches
}
