//! Full-screen terminal UI for the interactive session.
//!
//! A scrolling transcript region on top, a one-line status bar, a bordered
//! multi-line input box pinned to the bottom, and a slash-command popup that
//! opens the instant the line starts with '/' and filters as you type. Slash
//! commands and `/sql` execute and render into the transcript; live agent
//! streaming arrives in a later milestone. Non-TTY input uses a headless
//! executor, not this module. Rendering lives in `ui`.

mod agent;
mod atref;
mod complete;
mod dispatch;
mod exec;
mod fuzzy;
mod history;
mod input;
mod table;
mod transcript;
mod ui;

use super::session_resume::block_on;
use super::session_state::SessionState;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use agent::{Stream, StreamMsg};
use complete::Candidate;
use dispatch::Dispatch;
use history::History;
use input::InputBuffer;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use saya_agent::AgentEvent;
use saya_store::{FsSessionStore, SchemaStore, SessionStore, SqliteStateStore};
use std::cell::Cell;
use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use transcript::{BlockKind, Transcript};

/// Largest number of text rows the input box grows to before it stops expanding.
const MAX_INPUT_ROWS: usize = 6;

/// RAII guard: enters raw mode + the alternate screen on construction and
/// always restores the terminal on drop, including on panic/early return.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        )?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

/// Live slash-command popup state.
struct Menu {
    start: usize,
    end: usize,
    candidates: Vec<Candidate>,
    selected: usize,
}

/// A pending tool-approval request awaiting the user's y/n answer.
struct PendingApproval {
    tool: String,
    respond: oneshot::Sender<bool>,
}

/// A selectable list of saved sessions to resume.
struct Picker {
    entries: Vec<PickerEntry>,
    selected: usize,
}

/// One row in the session picker.
struct PickerEntry {
    id: String,
    label: String,
}

/// Interactive application state.
struct App {
    input: InputBuffer,
    transcript: Transcript,
    profiles: Vec<String>,
    menu: Option<Menu>,
    pending: Option<String>,
    stream: Option<Stream>,
    /// When the active request started streaming, for the elapsed timer.
    stream_started: Option<std::time::Instant>,
    /// The tool the agent is currently running, shown in the status bar.
    activity: Option<String>,
    spinner: usize,
    history: History,
    /// Transcript viewport (width, height) captured during the last render,
    /// so key-driven scrolling can clamp to the real size.
    viewport: Cell<(u16, u16)>,
    /// Set after a first Ctrl+C on an empty line; a second one exits.
    ctrl_c_armed: bool,
    /// `@table` / `@table.column` references from the active profiles' cached schema.
    at_refs: Vec<String>,
    /// A tool-approval request from the agent awaiting the user's answer.
    pending_approval: Option<PendingApproval>,
    /// Open session picker, if any.
    picker: Option<Picker>,
    /// A session id the user chose to resume, handled by the run loop.
    pending_resume: Option<String>,
    /// Whether the help overlay is shown.
    show_help: bool,
    runtime: Arc<RuntimeConfig>,
    state_db: SqliteStateStore,
    should_quit: bool,
}

impl App {
    fn new(profiles: Vec<String>, runtime: Arc<RuntimeConfig>, state_db: SqliteStateStore) -> Self {
        let mut transcript = Transcript::new();
        transcript.push(
            BlockKind::System,
            "Welcome to saya. Type a message and press Enter. Type / for commands, \
             Tab to accept a suggestion, ↑/↓ for history, Alt+Enter for a newline, Ctrl+C to quit.",
        );
        Self {
            input: InputBuffer::new(),
            transcript,
            profiles,
            menu: None,
            pending: None,
            stream: None,
            stream_started: None,
            activity: None,
            spinner: 0,
            history: History::load(),
            viewport: Cell::new((0, 0)),
            ctrl_c_armed: false,
            at_refs: Vec::new(),
            pending_approval: None,
            picker: None,
            pending_resume: None,
            show_help: false,
            runtime,
            state_db,
            should_quit: false,
        }
    }

    /// Opens the session picker with the most recent saved sessions, enriched
    /// with each session's profile, model, turn count, and relative age.
    fn open_session_picker(&mut self, store: &FsSessionStore) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let entries = match block_on(store.history()) {
            Ok(list) => list
                .into_iter()
                .take(20)
                .map(|entry| {
                    let when = relative_time(now_ms.saturating_sub(entry.modified_unix_ms));
                    let (profile, model, turns) = match block_on(store.load(&entry.id)) {
                        Ok(Some(session)) => (
                            session.profile.unwrap_or_else(|| "(no profile)".into()),
                            format!("{}/{}", session.provider, session.model),
                            session.turns.len(),
                        ),
                        _ => ("(no profile)".into(), "?".into(), 0),
                    };
                    PickerEntry {
                        label: format!("{when:<10}  {profile:<16}  {model:<24}  {turns} turn(s)"),
                        id: entry.id,
                    }
                })
                .collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        if entries.is_empty() {
            self.transcript
                .push(BlockKind::System, "No saved sessions to resume.");
            return;
        }
        self.picker = Some(Picker {
            entries,
            selected: 0,
        });
    }

    /// Moves the picker selection by `delta`, clamped.
    fn picker_move(&mut self, delta: isize) {
        if let Some(picker) = &mut self.picker {
            let len = picker.entries.len();
            if len == 0 {
                return;
            }
            let next = (picker.selected as isize + delta).clamp(0, len as isize - 1);
            picker.selected = next as usize;
        }
    }

    /// Confirms the picker selection, requesting a resume in the run loop.
    fn picker_confirm(&mut self) {
        if let Some(picker) = self.picker.take()
            && let Some(entry) = picker.entries.into_iter().nth(picker.selected)
        {
            self.pending_resume = Some(entry.id);
        }
    }

    /// Answers the pending tool-approval request and records the decision.
    fn answer_approval(&mut self, allow: bool) {
        if let Some(pending) = self.pending_approval.take() {
            let _ = pending.respond.send(allow);
            let verb = if allow { "Approved" } else { "Denied" };
            self.transcript
                .push(BlockKind::System, format!("{verb} tool: {}", pending.tool));
        }
    }

    /// Reloads `@`-reference names from the cached schema of the active and
    /// included profiles (best-effort; empty when nothing is cached).
    fn reload_at_refs(&mut self, state: &SessionState) {
        let mut names: Vec<&str> = Vec::new();
        if let Some(profile) = state.profile.as_deref() {
            names.push(profile);
        }
        names.extend(state.included_profiles.iter().map(String::as_str));
        let mut refs = Vec::new();
        for name in names {
            if let Ok(profile) = self.runtime.named_profile(name) {
                let identity = crate::profile_identity::profile_identity(
                    name,
                    profile,
                    &self.runtime.cache_scope,
                );
                if let Ok(Some(cached)) = block_on(self.state_db.get_schema(&identity)) {
                    refs.extend(atref::schema_refs(&cached.schema));
                }
            }
        }
        refs.sort();
        refs.dedup();
        self.at_refs = refs;
    }

    /// Scrolls the transcript by `delta` pages (negative = up), using the last
    /// rendered viewport height.
    fn scroll_pages(&mut self, up: bool) {
        let (_, height) = self.viewport.get();
        self.scroll_lines(up, (height as usize).saturating_sub(1).max(1));
    }

    /// Scrolls the transcript by `n` lines (for the mouse wheel).
    fn scroll_lines(&mut self, up: bool, n: usize) {
        let (width, height) = self.viewport.get();
        if up {
            self.transcript
                .scroll_up(n, width as usize, (height as usize).max(1));
        } else {
            self.transcript.scroll_down(n);
        }
    }

    /// Recalls the previous history entry into the input (Up).
    fn history_prev(&mut self) {
        if let Some(entry) = self.history.previous() {
            let entry = entry.to_string();
            self.input.set_text(entry);
            self.refresh_menu();
        }
    }

    /// Recalls the next history entry, or restores an empty line (Down).
    fn history_next(&mut self) {
        match self.history.next() {
            Some(entry) => self.input.set_text(entry.to_string()),
            None => self.input.clear(),
        }
        self.refresh_menu();
    }

    /// Whether an agent request is currently streaming.
    fn is_busy(&self) -> bool {
        self.stream.is_some()
    }

    /// Recomputes the popup: slash commands when the line starts with '/',
    /// otherwise `@table` references from the schema being typed at the cursor.
    fn refresh_menu(&mut self) {
        let cursor = self.input.cursor();
        let found = complete::slash_candidates(self.input.text(), &self.profiles)
            .or_else(|| atref::at_candidates(self.input.text(), cursor, &self.at_refs));
        self.menu = found.map(|(start, end, candidates)| Menu {
            start,
            end,
            candidates,
            selected: 0,
        });
    }

    /// Moves the popup selection by `delta`, clamped.
    fn menu_move(&mut self, delta: isize) {
        if let Some(menu) = &mut self.menu {
            let len = menu.candidates.len();
            if len == 0 {
                return;
            }
            let next = (menu.selected as isize + delta).clamp(0, len as isize - 1);
            menu.selected = next as usize;
        }
    }

    /// Replaces the completed token with the highlighted candidate. Completing a
    /// command word re-opens the popup for its argument; completing an argument
    /// value closes the popup so the next Enter submits.
    fn accept_selected(&mut self) {
        let mut completed_command = false;
        if let Some(menu) = &self.menu
            && let Some(candidate) = menu.candidates.get(menu.selected)
        {
            let chars: Vec<char> = self.input.text().chars().collect();
            let start = menu.start.min(chars.len());
            let end = menu.end.min(chars.len());
            let before: String = chars[..start].iter().collect();
            let after: String = chars[end..].iter().collect();
            completed_command = candidate.value.starts_with('/');
            let mut text = before;
            text.push_str(&candidate.value);
            if completed_command {
                text.push(' ');
            }
            text.push_str(&after);
            self.input.set_text(text);
        }
        if completed_command {
            self.refresh_menu();
        } else {
            self.menu = None;
        }
    }

    /// Inserts pasted text into the input without submitting (normalizing
    /// newlines), so multi-line pastes land in the box instead of running.
    fn paste(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        self.input.insert_str(&normalized);
        self.refresh_menu();
    }

    /// Captures the current line for dispatch and clears the input.
    fn submit(&mut self) {
        let line = self.input.text().trim_end().to_string();
        self.input.clear();
        self.menu = None;
        if line.is_empty() {
            return;
        }
        self.history.push(&line);
        if self.is_busy() {
            self.transcript
                .push(BlockKind::System, "Still working — press Esc to cancel.");
            self.transcript.scroll_to_bottom();
            return;
        }
        self.transcript.push(BlockKind::User, line.clone());
        self.transcript.scroll_to_bottom();
        self.pending = Some(line);
    }

    /// Starts streaming an agent prompt on a background thread.
    fn start_agent(&mut self, prompt: String, state: &SessionState) {
        let approval = state
            .approval_mode
            .parse()
            .unwrap_or(saya_agent::ApprovalPolicy::Ask);
        self.stream = Some(agent::start(
            self.runtime.clone(),
            prompt,
            approval,
            state.prompt_overrides(),
            state.provider_history(),
            self.state_db.clone(),
        ));
        self.stream_started = Some(std::time::Instant::now());
    }

    /// Drains any queued agent-stream messages into the transcript. Returns true
    /// when the request just finished (so the caller can persist the session).
    fn drain_stream(&mut self, state: &mut SessionState) -> bool {
        let mut messages = Vec::new();
        if let Some(stream) = self.stream.as_mut() {
            while let Ok(msg) = stream.rx.try_recv() {
                messages.push(msg);
            }
        }
        let prompt = self
            .stream
            .as_ref()
            .map(|s| s.prompt.clone())
            .unwrap_or_default();
        let mut finished = false;
        for msg in messages {
            match msg {
                StreamMsg::Event(event) => {
                    match &event {
                        AgentEvent::ToolRequested { name } => {
                            self.activity = Some(name.clone());
                        }
                        AgentEvent::AssistantText { .. } | AgentEvent::ToolCompleted { .. } => {
                            self.activity = None;
                        }
                        _ => {}
                    }
                    apply_event(&mut self.transcript, event);
                }
                StreamMsg::ApprovalRequest { tool, respond } => {
                    self.pending_approval = Some(PendingApproval { tool, respond });
                }
                StreamMsg::Done(result) => {
                    match result {
                        Ok(output) => state.record_turn(
                            prompt.clone(),
                            output.answer.clone(),
                            output.used_bounded_sql_query,
                            output.tool_metadata.clone(),
                        ),
                        Err(error) => self.transcript.push(BlockKind::Error, error),
                    }
                    finished = true;
                }
            }
        }
        if finished {
            self.stream = None;
            self.stream_started = None;
            self.activity = None;
        }
        // No forced scroll: when the user is at the bottom the newest lines show
        // automatically; when they've scrolled up to read, streaming leaves them.
        finished
    }

    /// Number of visible text rows the input box should show (clamped).
    fn input_rows(&self) -> usize {
        self.input.lines().len().clamp(1, MAX_INPUT_ROWS)
    }
}

/// Runs the full-screen TUI session. Returns the process exit code.
pub(crate) fn run(
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    state_db: &SqliteStateStore,
    format: RenderFormat,
    state: &mut SessionState,
) -> Result<i32, Box<dyn std::error::Error>> {
    let mut guard = TerminalGuard::new()?;
    let profiles = runtime
        .connections
        .profiles
        .keys()
        .cloned()
        .collect::<Vec<String>>();
    let mut app = App::new(profiles, Arc::new(runtime.clone()), state_db.clone());
    app.reload_at_refs(state);

    while !app.should_quit {
        let status = super::session_prompt::status_line(state);
        guard
            .terminal
            .draw(|frame| ui::draw(frame, &app, &status))?;

        if event::poll(Duration::from_millis(60))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key(&mut app, key.code, key.modifiers)
                }
                // Bracketed paste arrives as one event, so a multi-line paste
                // lands in the input instead of submitting on the first newline.
                Event::Paste(text) => app.paste(&text),
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => app.scroll_lines(true, 3),
                    MouseEventKind::ScrollDown => app.scroll_lines(false, 3),
                    _ => {}
                },
                _ => {}
            }
        }

        if app.is_busy() {
            app.spinner = app.spinner.wrapping_add(1);
            if app.drain_stream(state) {
                let _ = block_on(store.save(state.redacted()));
            }
        }

        if let Some(line) = app.pending.take() {
            match dispatch::dispatch(
                &line,
                &mut app.transcript,
                &app.profiles,
                state,
                runtime,
                store,
                format,
            ) {
                Dispatch::Quit => app.should_quit = true,
                // A command may have switched profiles; refresh @-references.
                Dispatch::Handled => app.reload_at_refs(state),
                Dispatch::Agent(prompt) => app.start_agent(prompt, state),
                Dispatch::OpenSessionPicker => app.open_session_picker(store),
            }
        }

        if let Some(id) = app.pending_resume.take() {
            let defaults = super::session_resume::SessionDefaults {
                provider: state.provider.clone(),
                model: state.model.clone(),
                allow_data_sharing: state.allow_data_sharing,
                approval_mode: state.approval_mode.clone(),
            };
            match super::session_resume::resume_session(store, &id, &defaults) {
                Ok(Some(loaded)) => {
                    *state = loaded;
                    app.reload_at_refs(state);
                    app.transcript
                        .push(BlockKind::System, format!("Resumed session {id}"));
                }
                Ok(None) => app
                    .transcript
                    .push(BlockKind::Error, format!("Session not found: {id}")),
                Err(error) => app.transcript.push(BlockKind::Error, error.to_string()),
            }
            let _ = block_on(store.save(state.redacted()));
        }
    }

    Ok(0)
}

/// Formats a millisecond age as a compact relative time (e.g. "3h ago").
fn relative_time(delta_ms: u128) -> String {
    let secs = delta_ms / 1000;
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Applies one streamed agent event to the transcript.
fn apply_event(transcript: &mut Transcript, event: AgentEvent) {
    match event {
        AgentEvent::AssistantText { text } => transcript.append_delta(BlockKind::Assistant, &text),
        AgentEvent::ToolRequested { name } => transcript.push(BlockKind::Tool, format!("→ {name}")),
        AgentEvent::ToolCompleted { name, summary } => {
            transcript.push(BlockKind::Tool, format!("✓ {name}: {summary}"))
        }
        AgentEvent::ToolDenied { name, reason } => {
            transcript.push(BlockKind::System, format!("✗ {name} denied: {reason}"))
        }
        AgentEvent::Complete => {}
    }
}

/// Applies one key press to the application state.
fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    // The help overlay is dismissed by any key.
    if app.show_help {
        app.show_help = false;
        return;
    }
    // F1 (or `?` on an empty line) opens the help overlay.
    if code == KeyCode::F(1) || (code == KeyCode::Char('?') && app.input.is_empty()) {
        app.show_help = true;
        return;
    }
    // The session picker captures navigation until confirmed or cancelled.
    if app.picker.is_some() {
        match code {
            KeyCode::Up => app.picker_move(-1),
            KeyCode::Down => app.picker_move(1),
            KeyCode::Enter => app.picker_confirm(),
            KeyCode::Esc => app.picker = None,
            _ => {}
        }
        return;
    }
    // A tool-approval modal captures input until answered.
    if app.pending_approval.is_some() {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => app.answer_approval(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.answer_approval(false),
            _ => {}
        }
        return;
    }
    // When the popup is open these keys drive it.
    if app.menu.is_some() {
        match code {
            KeyCode::Up => return app.menu_move(-1),
            KeyCode::Down => return app.menu_move(1),
            // Both Enter and Tab accept the highlighted suggestion.
            KeyCode::Tab | KeyCode::Enter => return app.accept_selected(),
            KeyCode::Esc => {
                app.menu = None;
                return;
            }
            _ => {}
        }
    }
    // Esc cancels an in-flight agent request.
    if code == KeyCode::Esc && app.is_busy() {
        if let Some(stream) = &app.stream {
            stream.cancel.cancel();
        }
        app.transcript.push(BlockKind::System, "Cancelling…");
        return;
    }
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let word = ctrl || alt;
    // Any key other than a bare Ctrl+C disarms the "press again to exit" state.
    let was_armed = app.ctrl_c_armed;
    app.ctrl_c_armed = false;
    match code {
        KeyCode::Char('c') if ctrl => {
            if app.is_busy() {
                if let Some(stream) = &app.stream {
                    stream.cancel.cancel();
                }
                app.transcript.push(BlockKind::System, "Cancelling…");
            } else if !app.input.is_empty() {
                app.input.clear();
                app.menu = None;
            } else if was_armed {
                app.should_quit = true;
            } else {
                app.ctrl_c_armed = true;
                app.transcript
                    .push(BlockKind::System, "Press Ctrl+C again to exit.");
            }
            return;
        }
        KeyCode::Char('d') if ctrl && app.input.is_empty() => return app.should_quit = true,
        KeyCode::Char('a') if ctrl => app.input.move_home(),
        KeyCode::Char('e') if ctrl => app.input.move_end(),
        KeyCode::Char('k') if ctrl => app.input.kill_to_line_end(),
        KeyCode::Char('u') if ctrl => app.input.kill_to_line_start(),
        KeyCode::Char('w') if ctrl => app.input.delete_word_left(),
        KeyCode::Char(c) if !ctrl => app.input.insert_char(c),
        KeyCode::Enter if alt || mods.contains(KeyModifiers::SHIFT) => app.input.insert_newline(),
        KeyCode::Enter => return app.submit(),
        KeyCode::Backspace => app.input.backspace(),
        KeyCode::Delete => app.input.delete(),
        KeyCode::Left if word => app.input.move_word_left(),
        KeyCode::Right if word => app.input.move_word_right(),
        KeyCode::Left => app.input.move_left(),
        KeyCode::Right => app.input.move_right(),
        KeyCode::Home => app.input.move_home(),
        KeyCode::End => app.input.move_end(),
        KeyCode::Up => return app.history_prev(),
        KeyCode::Down => return app.history_next(),
        KeyCode::PageUp => return app.scroll_pages(true),
        KeyCode::PageDown => return app.scroll_pages(false),
        _ => return,
    }
    // A real edit or cursor move ends history navigation, so the next Up starts fresh.
    app.history.reset();
    app.refresh_menu();
}
