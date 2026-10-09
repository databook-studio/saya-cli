use super::*;

// -- assertions on what the child reported ------------------------------

/// The first line carrying `prefix`, if the child reported one.
pub(super) fn report_line<'a>(report: &'a str, prefix: &str) -> &'a str {
    report
        .lines()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("the child must report {prefix:?}: {report}"))
}

/// Decodes the helper's `arg<i>=<hex>` echo back into the argv the child
/// actually received (element 0 is the program path).
pub(super) fn received_argv(report: &str) -> Vec<String> {
    let mut received = Vec::new();
    for line in report.lines() {
        let Some((index, hex)) = line
            .strip_prefix("arg")
            .and_then(|rest| rest.split_once('='))
        else {
            continue;
        };
        assert_eq!(
            index.parse::<usize>().expect("arg index"),
            received.len(),
            "argv elements arrive in order"
        );
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex byte"))
            .collect::<Vec<u8>>();
        received.push(String::from_utf8(bytes).expect("the composed argv is UTF-8"));
    }
    assert_eq!(
        received.len(),
        15,
        "the child received the composed argv: {report}"
    );
    received
}

/// FNV-1a, byte-identical to the helper's.
pub(super) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Every regular file under `root`, at any depth.
pub(super) fn run_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}
