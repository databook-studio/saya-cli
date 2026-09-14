use crate::approval_facts::ApprovalFacts;
use crate::grant_token::{TurnPrimary, grant_token, session_answers_line};
use saya_agent::{
    ApprovalChoice, ApprovalDecision, ApprovalPolicy, SessionGrants, SessionPolicy, ToolDefinition,
};
use saya_store::SessionJournal;
use std::sync::Arc;

pub(crate) struct TerminalApproval {
    policy: SessionPolicy,
    can_prompt: bool,
    /// The turn's primary connection, bound by the turn that owns this
    /// decider; the SQL family's suggestion names it when the call names
    /// no connection. Unbound — a run's construction — suggests no token.
    primary: TurnPrimary,
    /// The composition facts the prompt may state. Built from the session
    /// universe (or the resolved config on the one-shot ask path); the
    /// prompt states only these, never prose.
    facts: ApprovalFacts,
    /// The session journal, when this decider belongs to a session — a
    /// `[s]` answer's new grant is journalled there, before the call it
    /// allowed runs. `None` — the one-shot ask and the headless shapes —
    /// records grants with no journal: there is no session to journal for.
    journal: Option<Arc<SessionJournal>>,
}

impl TerminalApproval {
    pub(crate) fn new(
        policy: ApprovalPolicy,
        can_prompt: bool,
        primary: TurnPrimary,
        facts: ApprovalFacts,
    ) -> Self {
        Self {
            policy: SessionPolicy::new(policy),
            can_prompt,
            primary,
            facts,
            journal: None,
        }
    }

    /// The headless run's decider (U4: the same engine, frozen): built over a
    /// [`SessionPolicy::frozen`] seeded from the run's `--allow` tokens — the
    /// stated scopes are the approval — so a seed pre-answers the asks it
    /// names and everything else an `ask` mode would raise denies with the
    /// engine's own reason. `facts` is the run's own composition (built in
    /// `commands/run` from the approved scopes and the runner wiring), so a
    /// seed pre-answers exactly the calls the composition carries (U8): a
    /// token the composition cannot honour is never suggested, on this
    /// surface either. It prompts nothing and records nothing: a headless
    /// session grant is impossible, not merely unused. The primary stays
    /// unbound — the run's fail-closed rule: only a call that names its
    /// connection suggests a token, never a guessed one.
    pub(crate) fn frozen(mode: ApprovalPolicy, seeds: &[String], facts: ApprovalFacts) -> Self {
        Self {
            policy: SessionPolicy::frozen(mode, seeds),
            can_prompt: false,
            primary: TurnPrimary::default(),
            facts,
            journal: None,
        }
    }

    /// Built over the session's one approval policy — the hoisted instance a
    /// session's turns clone, so a grant recorded through this decider's ask
    /// is in force for every later turn of the same session. The primary is
    /// the turn's handle: the turn binds the registry's primary into it
    /// before the model runs. `facts` are the session composition's prompt
    /// facts — what this decider's prompts may state about the session. The
    /// session's journal rides along: a `[s]` answer's new grant is written
    /// there before this call is allowed to run.
    pub(crate) fn from_session(
        policy: SessionPolicy,
        can_prompt: bool,
        primary: TurnPrimary,
        facts: ApprovalFacts,
        journal: Option<Arc<SessionJournal>>,
    ) -> Self {
        Self {
            policy,
            can_prompt,
            primary,
            facts,
            journal,
        }
    }
}

/// The prompt shown when an `Ask` approval needs the user: the per-call fact
/// body (`approval_facts::call_facts`) — the containment that makes the call
/// safe, the bounds that cap it, the session's grant history — followed by
/// the answers line; for a call with no facts worth showing, a generic
/// sentence naming the tool, so nothing is ever approved under a sentence it
/// does not match. The TUI's modal renders the same body and answers line
/// (`interactive/tui/ui/panels`), so the two frontends cannot state
/// different facts.
pub(crate) fn approval_prompt(
    tool: &ToolDefinition,
    arguments: &serde_json::Value,
    grant: Option<&str>,
    facts: &ApprovalFacts,
    primary: Option<&str>,
    grants: Option<&SessionGrants>,
) -> String {
    let answers = session_answers_line(grant);
    match crate::approval_facts::call_facts(&tool.name, arguments, grant, facts, primary, grants) {
        Some(body) => format!("{body}\n{answers} "),
        None => format!("Run tool `{}`? {answers} ", tool.name),
    }
}

/// The user's typed answer mapped onto a [`ApprovalChoice`]. The habit and the
/// script keep their meaning — `y`/`yes` allow once, `n`/`no` deny — `a` is
/// the allow-once answer's key, `s` grants exactly the offered token (and is
/// an unoffered deny when no token exists), and anything unrecognised denies.
pub(crate) fn terminal_choice(answer: &str, grant: Option<&str>) -> ApprovalChoice {
    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" | "a" => ApprovalChoice::AllowOnce,
        "s" => match grant {
            Some(token) => ApprovalChoice::AllowSession {
                token: token.to_owned(),
            },
            None => ApprovalChoice::Deny,
        },
        _ => ApprovalChoice::Deny,
    }
}

#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for TerminalApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool {
        let primary = self.primary.get();
        let grant = grant_token(&tool.name, arguments, primary.as_deref(), &self.facts);
        match self.policy.resolve(&tool.effect, grant.as_deref()) {
            ApprovalDecision::Allow => true,
            ApprovalDecision::Deny { .. } => false,
            ApprovalDecision::Ask if !self.can_prompt => false,
            ApprovalDecision::Ask => {
                use std::io::{self, IsTerminal, Write};
                if !io::stdin().is_terminal() {
                    return false;
                }
                let primary = primary.as_deref();
                eprint!(
                    "{}",
                    approval_prompt(
                        tool,
                        arguments,
                        grant.as_deref(),
                        &self.facts,
                        primary,
                        Some(self.policy.grants()),
                    )
                );
                let _ = io::stderr().flush();
                let mut answer = String::new();
                if !(io::stdin().read_line(&mut answer).is_ok()) {
                    return false;
                }
                let choice = terminal_choice(&answer, grant.as_deref());
                // Only "allow for this session" records anything; the grant
                // lands in the session's one policy, so it outlives the turn,
                // and a *new* grant is journalled there before this call is
                // allowed to run — the shared operation, one wording. A
                // failed journal write changes no consent: it is said on
                // stderr, the prompt's own channel, and the session carries
                // on.
                let (_, warning) = crate::interactive::session_grants::record_prompt_answer(
                    &self.policy,
                    &choice,
                    self.journal.as_deref(),
                );
                if let Some(warning) = warning {
                    eprintln!("{warning}");
                }
                !matches!(choice, ApprovalChoice::Deny)
            }
        }
    }
}
