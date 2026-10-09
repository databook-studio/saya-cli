#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TerminalEvent {
    AssistantText {
        text: String,
    },
    ToolRequested {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        /// The tool's **declared effect**, carried from the event the loop
        /// emitted so the text line's effect claim is derived from the same
        /// declaration the approval gate reads — never from the tool's name.
        /// Absent only when no declaration exists (an unknown tool, which
        /// cannot run), and skipped on the wire then, so a stream that never
        /// declared a tool keeps the shape it always had.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effect: Option<ToolEffect>,
    },
    ToolCompleted {
        name: String,
        summary: String,
    },
    ToolDenied {
        name: String,
        reason: String,
    },
    /// What memory **supplied** to the turn, emitted once before the provider
    /// call. Carries the outcome, the supplied contracts (claim DTOs,
    /// no opaque identity), and the count the bounds dropped. Text is shaped in
    /// [`render_memory`]; JSON/NDJSON fall out of the serde derive.
    KnowledgeSupplied {
        outcome: KnowledgeOutcome,
        contracts: Vec<SuppliedContractDto>,
        dropped_by_bounds: usize,
    },
    /// One fact SAYA came away from the turn knowing (`AgentEvent::KnowledgeProposed`).
    /// Emitted once per learned claim, after the answer. Text is shaped in
    /// [`render_learned`] and carries no claim id — learning is not something the
    /// user asked for, so it must not hand them a hash to manage. JSON/NDJSON keep
    /// the DTO whole, id included, for machine consumers.
    KnowledgeLearned {
        claim: ProposedClaimDto,
    },
    /// A confirmed claim the turn's SQL **contradicted**. Emitted at
    /// most once per turn, after the loop, carrying every finding the detector
    /// raised. The finding says the SQL **referenced** columns, never that it
    /// **used** them — the extractor cannot prove role. Text is shaped in
    /// [`render_memory`]; JSON/NDJSON fall out of the serde derive.
    KnowledgeOverridden {
        findings: Vec<OverrideFindingDto>,
    },
    /// Post-turn extraction was skipped after the turn succeeded — no memory
    /// was recorded, and the line says so. Trails the answer.
    /// Text is shaped in [`render_memory`]; JSON/NDJSON fall out of the serde
    /// derive.
    KnowledgeLearningSkipped {
        reason: LearningSkipReason,
    },
    /// The extraction circuit breaker tripped this turn — no more extraction
    /// requests this session (`AgentEvent::KnowledgeLearningDisabled`).
    /// Trails the answer, after `KnowledgeLearningSkipped`. Text is shaped
    /// in [`render_memory`]; JSON/NDJSON fall out of the serde derive.
    KnowledgeLearningDisabled {
        model: String,
        misses: u32,
    },
    /// The model stopped to ask the user one focused question instead of
    /// assuming (`AgentEvent::ClarificationNeeded`, B3c). The turn ends here;
    /// the user's next message answers it. Text is shaped in
    /// [`render_clarification`] — the same block the TUI transcript pushes;
    /// JSON/NDJSON fall out of the serde derive.
    ClarificationNeeded {
        question: String,
        options: Vec<String>,
    },
    Complete,
    /// The ONE terminal result of a scripted (`--turn-file`) run, emitted
    /// last on stdout under json/ndjson: how the turn ended, the process
    /// exit code (0 completed — an explicit `/exit` included — and 5
    /// otherwise, cancelled and budget-stopped included, unlike `ask`/`run`),
    /// and the honest tool tally: the calls that completed plus the
    /// requested-but-unfinished calls whose effects the stream never
    /// settled. Text mode says nothing here; its unfinished calls warn as a
    /// separate stderr diagnostic (render_turn).
    TurnResult {
        status: TurnStatus,
        exit_code: i32,
        tools_completed: usize,
        tools_unfinished: Vec<String>,
    },
    /// The provider stream failed mid-answer and the turn is being retried
    /// (`AgentEvent::TurnReset`). The partial answer printed so far is
    /// discarded; the retry re-streams the full answer. Carried on the
    /// JSON/NDJSON stream under its own tag so a machine consumer can replace
    /// the text it accumulated instead of appending; the text adapter prints a
    /// one-line notice, because an answer that silently restarts mid-stream
    /// would read as the model repeating itself.
    TurnReset,
    /// The SQL the model designated as the answering query for the turn.
    /// Carried on the NDJSON stream so a harness can pair the answer with its
    /// query; silent in the text adapter, where the SQL was already shown when
    /// the query ran.
    AnswerDesignated {
        sql: String,
    },
    /// The consensus decision over multiple candidate attempts — emitted once,
    /// after all attempts, whenever more than one attempt ran. Carries the
    /// winning SQL (or `None` when the attempts disagreed with no tie-break)
    /// and the vote tallies. Text is shaped inline; JSON/NDJSON fall out of the
    /// serde derive.
    ConsensusDecided {
        sql: Option<String>,
        attempts: usize,
        voted: usize,
        votes: usize,
        margin: usize,
        tied: bool,
        probe_broke_tie: bool,
    },
    /// The token counts one provider call reported (`AgentEvent::Usage`),
    /// named by `call` so a consumer can keep the answering rounds' cost apart
    /// from the extraction call's. Carried on the JSON/NDJSON stream for
    /// machine consumers; the text adapter renders nothing, because the
    /// interactive surfaces for it already exist (the per-turn token line and
    /// `/usage`) and a pipe's reader has the answer above it.
    Usage {
        call: UsageCall,
        usage: TokenUsage,
    },
    /// Nonterminal provider-attempt recovery progress. Text names the stable
    /// phase and local reason; JSON/NDJSON preserve the typed fields.
    ProviderRecovery {
        phase: ProviderRecoveryPhase,
        reason: ProviderRecoveryReason,
        attempt: u8,
        limit: u8,
    },
    /// Usage known from an attempt whose content was discarded. Machine output
    /// carries it; text stays silent like successful per-call usage.
    FailedAttemptUsage {
        usage: TokenUsage,
    },
    Result {
        message: String,
    },
    QueryResult {
        result: QueryResult,
    },
    Schema {
        schema: SchemaTree,
    },
    NotImplemented {
        feature: String,
    },
    Diagnostic {
        message: String,
    },
    /// One task's `config doctor --task` readiness. Text is shaped in
    /// [`render_doctor`]; JSON/NDJSON fall out of the serde derive.
    DoctorTask {
        task: String,
        ready: bool,
        checks: Vec<TaskCheck>,
    },
    Error {
        message: String,
    },
    /// Resume refused to repeat a journaled incomplete step because its
    /// declared effects may already have happened.
    ResumeUncertain {
        run_id: String,
        /// One-based plan step number.
        step: usize,
        /// Unique effect codes returned by the harness; at most one per code.
        effects: Vec<ResumeEffectCode>,
        /// Redacted, character-bounded plan goal.
        goal: String,
    },
    ContractList {
        contracts: Vec<ContractView>,
    },
    ContractShow {
        contract: ContractView,
    },
    ContractChanged {
        claim_id: String,
        action: String,
        status: String,
    },
    ContractRemembered {
        /// Carried for machine consumers only. The text renderer never prints
        /// it: a 64-character hash is the system's business, and a script that
        /// remembers then forgets still needs a handle without a second call.
        claim_id: String,
        object: String,
        kind: String,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        column: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<String>,
        action: String,
        status: String,
    },
    ContractQueue {
        items: Vec<ContractQueueItemView>,
    },
    ContractImport {
        report: ContractImportView,
    },
    ContractExport {
        report: ContractExportView,
    },
}
