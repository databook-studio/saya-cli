pub(crate) struct TerminalSink {
    format: RenderFormat,
    text_open: Mutex<bool>,
    group: Mutex<TextGroupState>,
    answering_usage: AnsweringReceipts,
    /// The shared tool tally this sink records requested/completed events
    /// into — the scripted run's one terminal result reports it. `None` for
    /// sinks no tally asked for.
    tally: Option<TallyHandle>,
}
impl TerminalSink {
    pub(crate) fn new(format: RenderFormat) -> Self {
        Self {
            format,
            text_open: Mutex::new(false),
            group: Mutex::new(TextGroupState::new()),
            answering_usage: AnsweringReceipts::default(),
            tally: None,
        }
    }

    /// Records this sink's tool events into a shared tally the caller reads
    /// once the turn settles.
    pub(crate) fn with_tally(mut self, tally: TallyHandle) -> Self {
        self.tally = Some(tally);
        self
    }

    pub(crate) fn take_answering_usage(&self) -> Option<saya_agent::TokenUsage> {
        self.answering_usage.take()
    }
}

#[async_trait]
impl AgentEventSink for TerminalSink {
    async fn emit(&self, event: AgentEvent) {
        self.answering_usage.record(&event);
        if let Some(tally) = &self.tally {
            tally.lock().expect("tool tally").record(&event);
        }
        let rendered = render_text_stream(
            event,
            self.format,
            &mut self.text_open.lock().expect("terminal state"),
            &mut self.group.lock().expect("terminal state"),
        );
        print!("{}", rendered.stdout);
        eprint!("{}", rendered.stderr);
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
    }
}
