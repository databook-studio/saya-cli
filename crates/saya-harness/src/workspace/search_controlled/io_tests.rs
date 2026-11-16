//! Real held-file metadata then external truncation, at the actual stream owner.
use super::*;
#[test]
fn short_held_file_read_is_unavailable() {
    let root = std::env::temp_dir().join(format!(
        "saya-search-short-read-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let target = root.join("entry.txt");
    std::fs::write(&target, b"declared original bytes\n").unwrap();
    let file = std::fs::File::open(&target).unwrap();
    let declared = file.metadata().unwrap().len();
    std::fs::write(&target, b"").unwrap();
    let token = saya_agent::CancellationToken::new();
    let control = Control {
        token: &token,
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(2),
    };
    let mut budget = ReadBudget {
        used: 0,
        maximum: 1024,
    };
    let outcome = budget
        .read_opened(file, declared, 1024, &control)
        .unwrap()
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        matches!(outcome, FileRead::Unavailable),
        "a real shortened held-file snapshot must not become complete empty content"
    );
}
