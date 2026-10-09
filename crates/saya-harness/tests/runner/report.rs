    // -- assertions on what the child left behind ---------------------------

    /// `true` when no member of the child's process group remains: the
    /// group id is the child's pid (`process_group(0)`), so signal 0 to the
    /// group probing membership returns ESRCH exactly when everything died.
    fn group_gone(pgid: u32) -> bool {
        let result = unsafe { libc::killpg(pgid as libc::pid_t, 0) };
        result == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
    fn wait_group_gone(pgid: u32) -> bool {
        let start = Instant::now();
        loop {
            if group_gone(pgid) {
                return true;
            }
            if start.elapsed() > Duration::from_secs(10) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Decodes the helper's `argv` mode output back into the elements the
    /// child actually received.
    fn received_argv(outcome: &ProgramOutcome) -> Vec<String> {
        assert_eq!(
            outcome.exit_code,
            Some(0),
            "the argv dumper must exit 0: stderr {:?}",
            outcome.stderr.text
        );
        let mut received = Vec::new();
        let mut argc = None;
        for line in outcome.stdout.text.lines() {
            if let Some(count) = line.strip_prefix("argc=") {
                argc = Some(count.parse::<usize>().expect("argc is a number"));
            } else if let Some((index, hex)) = line
                .strip_prefix("arg")
                .and_then(|rest| rest.split_once('='))
            {
                assert_eq!(
                    index.parse::<usize>().expect("arg index"),
                    received.len(),
                    "argv elements arrive in order"
                );
                let bytes = (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex byte"))
                    .collect::<Vec<u8>>();
                received.push(String::from_utf8(bytes).expect("the corpus is UTF-8"));
            }
        }
        assert_eq!(
            argc,
            Some(received.len()),
            "argc must match the number of decoded elements: {received:?}"
        );
        received
    }
