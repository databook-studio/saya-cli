// The pty harness: one `saya` child on a pseudo-terminal, the screen vt100
// parses from its byte stream, and the two readings a report needs of it — the
// plain rows, and the rows' foreground colour runs.

struct Cap {
    writer: Box<dyn Write + Send>,
    parser: vt100::Parser,
    rx: Receiver<std::io::Result<Vec<u8>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Drop for Cap {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.try_wait();
    }
}

impl Cap {
    fn send(&mut self, text: &str) {
        let _ = self.writer.write_all(text.as_bytes());
        let _ = self.writer.flush();
    }

    fn pump(&mut self) {
        if let Ok(Ok(bytes)) = self.rx.recv_timeout(READ)
            && !bytes.is_empty()
        {
            self.parser.process(&bytes);
        }
    }

    fn screen(&self) -> String {
        rows_text(self.parser.screen())
    }

    fn wait_for(&mut self, needle: &str) {
        let start = Instant::now();
        while !self.screen().contains(needle) {
            assert!(
                start.elapsed() < DEADLINE,
                "{needle:?} never appeared; screen:\n{}",
                self.screen()
            );
            self.pump();
        }
    }

    /// Pumps until the screen's text has not changed for [`SETTLE`].
    fn settle(&mut self) {
        let (mut previous, mut changed, start) = (String::new(), Instant::now(), Instant::now());
        while changed.elapsed() < SETTLE && start.elapsed() < DEADLINE {
            self.pump();
            let now = self.screen();
            if now != previous {
                previous = now;
                changed = Instant::now();
            }
        }
    }

    fn print(&self, tag: &str) {
        println!("=== {tag} ===");
        println!("{}", self.screen());
        println!("--- {tag}: colour runs ---");
        println!("{}", colour_runs(self.parser.screen()));
    }
}

fn rows_text(screen: &vt100::Screen) -> String {
    let cols = screen.size().1;
    let mut out = String::new();
    for row in screen.rows(0, cols) {
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

/// Each row's runs of non-default foreground as `rgb(r,g,b):text`, so paint is
/// readable in a text report.
fn colour_runs(screen: &vt100::Screen) -> String {
    let (rows, cols) = screen.size();
    let mut out = String::new();
    for row in 0..rows {
        let mut runs: Vec<String> = Vec::new();
        let (mut run, mut hue) = (String::new(), None);
        for column in 0..cols {
            let Some(cell) = screen.cell(row, column) else {
                continue;
            };
            let fg = match cell.fgcolor() {
                vt100::Color::Default => None,
                other => Some(other),
            };
            if fg != hue && !run.is_empty() {
                runs.push(format!("{}{run}", name(hue)));
                run.clear();
            }
            hue = fg;
            run.push_str(cell.contents());
        }
        if !run.trim().is_empty() {
            runs.push(format!("{}{run}", name(hue)));
        }
        if runs.iter().any(|run| !run.starts_with("plain:")) {
            out.push_str(&format!("{row:>2} {}\n", runs.join(" · ")));
        }
    }
    out
}

fn name(hue: Option<vt100::Color>) -> String {
    match hue {
        Some(vt100::Color::Rgb(r, g, b)) => format!("rgb({r},{g},{b}):"),
        Some(vt100::Color::Idx(index)) => format!("idx({index}):"),
        _ => "plain:".to_string(),
    }
}
