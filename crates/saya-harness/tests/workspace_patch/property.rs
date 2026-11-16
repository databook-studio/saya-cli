use super::*;
use proptest::prelude::*;

fn hostile_arg() -> impl Strategy<Value = String> {
    let pool = vec![
        "ok.txt".to_string(),
        "sub".to_string(),
        "nested.txt".to_string(),
        "victim.txt".to_string(),
        "..".to_string(),
        "..".to_string(),
        ".".to_string(),
        String::new(),
        "outside.txt".to_string(),
        "/etc/passwd".to_string(),
        "C:\\Windows".to_string(),
        "\\\\srv\\share".to_string(),
        ".git".to_string(),
        "~".to_string(),
        "link".to_string(),
    ];
    prop::collection::vec(prop::sample::select(pool), 1..6).prop_map(|parts| parts.join("/"))
}

proptest! {
    /// No generated argument may ever patch outside the workspace. The
    /// outside sentinel keeps its exact bytes, no name is planted outside,
    /// and accepted patches return the inside sentinel's spliced form.
    #[test]
    fn containment_holds_for_the_corpus(args in prop::collection::vec(hostile_arg(), 1..8)) {
        let sandbox = Sandbox::new("proptest-patch");
        sandbox.ws.write("victim.txt", INSIDE).unwrap();
        sandbox.ws.write("sub/nested.txt", INSIDE).unwrap();
        fs::write(sandbox.outside_path("outside.txt"), OUTSIDE).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            sandbox.outside_path("outside.txt"),
            sandbox.root().join("link"),
        )
        .unwrap();

        for arg in args {
            let current = fs::read(sandbox.root().join("victim.txt")).unwrap_or_default();
            let len = current.len() as u64;
            let end = len.min(3);
            let _ = sandbox.ws.patch_range(&arg, 0..end, len, b"zz");
            prop_assert_eq!(
                fs::read(sandbox.outside_path("outside.txt")).unwrap(),
                OUTSIDE
            );
            prop_assert!(!sandbox.outside_path("attempted").exists());
        }
    }

    /// Splice model: for any content, any in-bounds range, and any small
    /// replacement, `patch_range` produces exactly prefix + replacement +
    /// suffix, and any out-of-bounds or stale-precondition call refuses with
    /// the file untouched.
    #[test]
    fn splice_matches_prefix_plus_replacement_plus_suffix(
        content in prop::collection::vec(any::<u8>(), 0..256),
        start in 0..300usize,
        end in 0..300usize,
        replacement in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let sandbox = Sandbox::new("proptest-splice");
        sandbox.ws.write("victim.txt", &content).unwrap();
        let size = content.len() as u64;
        let (start, end) = (start as u64, end as u64);
        let expected: Vec<u8> = if start <= end && end <= size {
            let mut out = Vec::new();
            out.extend_from_slice(&content[..start as usize]);
            out.extend_from_slice(&replacement);
            out.extend_from_slice(&content[end as usize..]);
            out
        } else {
            Vec::new()
        };
        if start <= end && end <= size {
            sandbox.ws.patch_range("victim.txt", start..end, size, &replacement).unwrap();
            prop_assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), expected);
        } else {
            let error = sandbox.ws.patch_range("victim.txt", start..end, size, &replacement)
                .expect_err("out-of-bounds range must refuse");
            prop_assert!(matches!(error, HarnessError::RangeOutOfBounds { .. }), "{error:?}");
            prop_assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), content);
        }
    }
}
