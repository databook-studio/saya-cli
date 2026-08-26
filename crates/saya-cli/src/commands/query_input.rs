use std::{
    fs,
    io::{IsTerminal as _, Read},
    path::PathBuf,
};

pub(super) fn input(
    value: Option<String>,
    file: Option<PathBuf>,
) -> Result<String, Box<dyn std::error::Error>> {
    match (value, file) {
        (Some(value), None) => Ok(value),
        (None, Some(path)) => Ok(fs::read_to_string(path)?),
        (Some(_), Some(_)) => Err("provide a prompt or --file, not both".into()),
        // Piped input (`pbpaste | saya ask`, `echo sql | saya query`) is the
        // scripting path: slurp stdin instead of demanding an argument.
        (None, None) if !std::io::stdin().is_terminal() => {
            let mut buffer = String::new();
            std::io::stdin().read_to_string(&mut buffer)?;
            let trimmed = buffer.trim();
            if trimmed.is_empty() {
                Err("a prompt or --file is required".into())
            } else {
                Ok(trimmed.to_string())
            }
        }
        (None, None) => Err("a prompt or --file is required".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_inputs_win_and_conflicts_error() {
        assert_eq!(input(Some("hi".into()), None).unwrap(), "hi");
        let path = std::env::temp_dir().join(format!("saya-qin-{}.txt", std::process::id()));
        std::fs::write(&path, "from file").unwrap();
        assert_eq!(input(None, Some(path.clone())).unwrap(), "from file");
        let _ = std::fs::remove_file(&path);
        assert!(input(Some("a".into()), Some("b".into())).is_err());
        // In test harnesses stdin is non-TTY (pipe/null); empty stdin must
        // still demand an argument rather than submitting "".
    }
}
