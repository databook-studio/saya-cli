#[cfg(test)]
mod turn_file_tests {
    use super::*;

    #[test]
    fn oversized_turn_file_is_refused_without_echoing_contents() {
        let path = std::env::temp_dir().join(format!(
            "saya-turn-file-over-limit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sentinel = "turn-file-secret-sentinel";
        let mut bytes = vec![b'x'; crate::commands::query_input::FILE_BYTE_LIMIT + 1];
        bytes[..sentinel.len()].copy_from_slice(sentinel.as_bytes());
        std::fs::write(&path, bytes).unwrap();
        let error = read_turn_file(&path).expect_err("oversized turn files must be refused");
        assert!(error.contains(&crate::commands::query_input::FILE_BYTE_LIMIT.to_string()));
        assert!(!error.contains(sentinel));
        let _ = std::fs::remove_file(path);
    }
}
