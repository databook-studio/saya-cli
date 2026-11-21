/// One plain message: a turn of the session's run.
///
/// The run is composed and opened on the session's first message and every
/// later message is a turn of that same run — same epoch, lease and provider.
/// What the turn prints is the front's: the terminal sink streams the answer and
/// its tool steps, and the settled turn says its own wording (the budget
/// sentence, the pause line, `Request cancelled.`). What stays here is the
/// loop's bookkeeping — the tally a scripted run reports, the record's run link,
/// the save — and the process's turn outcome.
fn handle_message(
    line: &str,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let format = ctx.format;
    let terminal = ctx.terminal;
    let store = ctx.store;
    let state_db = ctx.state_db;
    let runtime = Arc::clone(&ctx.runtime);
    let save_refused = &mut ctx.save_refused;
    let state = &mut *ctx.state;
    let session = &mut *ctx.session;
    let approval = state
        .approval_mode_parsed()
        .map_err(|error: saya_agent::ApprovalPolicyParseError| error.to_string())?;
    let said = match ctx.chat.as_ref() {
        Some(chat) => chat.send(line.to_owned())?,
        None => {
            // The session's first message: the run is composed over the folder
            // the session was started in and opened before this turn.
            let folder = session
                .root()
                .map(std::path::Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok())
                .ok_or("the session has no folder to open a run over")?;
            let chat = LineChat::open(LineSession {
                session: state.id.clone(),
                folder,
                runtime,
                store: state_db.clone(),
                mode: approval,
                overrides: state.prompt_overrides(),
                format,
                can_prompt: terminal,
            })?;
            // The record names the run the session belongs to: lineage only.
            state.run = chat.run_link();
            let said = chat.send(line.to_owned())?;
            ctx.chat = Some(chat);
            said
        }
    };
    let tally = ctx
        .chat
        .as_ref()
        .map(LineChat::tally)
        .unwrap_or_default();
    // A pause is not a failure: the next message continues from where the guard
    // stopped the turn. Every other stop keeps today's exit contract — a
    // cancelled turn and a stop that ended it errored the run.
    let (status, outcome) = match &said {
        Some(ChatOutcome::Stopped(_)) => (
            prose_status(ProseOutcome::Failed, true),
            TurnOutcome::Errored,
        ),
        Some(ChatOutcome::Failed { .. }) => {
            (prose_status(ProseOutcome::Failed, false), TurnOutcome::Errored)
        }
        Some(ChatOutcome::Cancelled) => (
            prose_status(ProseOutcome::Cancelled, false),
            TurnOutcome::Errored,
        ),
        Some(ChatOutcome::Paused { .. }) | Some(ChatOutcome::Answered) | None => (
            prose_status(ProseOutcome::Completed, false),
            TurnOutcome::Completed,
        ),
    };
    ctx.last_turn = Some(TurnSummary { status, tally });
    if let Some(outcome) = persist(state, store, format, save_refused)? {
        return Ok(outcome);
    }
    Ok(outcome)
}
