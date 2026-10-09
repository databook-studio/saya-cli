fn text_event(event: &TerminalEvent) -> Rendered {
    let rendered = match event {
        TerminalEvent::Diagnostic { message } | TerminalEvent::Error { message } => Rendered {
            stdout: String::new(),
            stderr: format!("{message}\n"),
        },
        TerminalEvent::ResumeUncertain {
            run_id,
            step,
            effects,
            goal,
        } => Rendered {
            stdout: String::new(),
            stderr: format!(
                "run {run_id} paused: step {step} ({}) may have produced effects before completion was journaled: {}. Reconcile them before retrying with `saya run resume {run_id} --retry-incomplete`.\n",
                sanitize_terminal(goal),
                effects
                    .iter()
                    .map(|effect| resume_effect_label(*effect))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
        TerminalEvent::AssistantText { text } => render_delta::text(text),
        TerminalEvent::ToolRequested {
            name,
            detail,
            effect,
        } => Rendered {
            stdout: tool_requested_text(name, detail.as_deref(), effect.as_ref()),
            stderr: String::new(),
        },
        TerminalEvent::ToolCompleted { name, summary } => Rendered {
            stdout: format!("{name}: {summary}\n"),
            stderr: String::new(),
        },
        TerminalEvent::ToolDenied { name, reason } => Rendered {
            stdout: format!("Approval denied for {name}: {reason}\n"),
            stderr: String::new(),
        },
        TerminalEvent::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        } => Rendered {
            stdout: render_memory::knowledge_supplied_text(*outcome, contracts, *dropped_by_bounds),
            stderr: String::new(),
        },
        TerminalEvent::KnowledgeLearned { claim } => Rendered {
            stdout: render_learned::knowledge_learned_text(claim),
            stderr: String::new(),
        },
        TerminalEvent::KnowledgeOverridden { findings } => Rendered {
            stdout: render_memory::knowledge_overridden_text(findings),
            stderr: String::new(),
        },
        TerminalEvent::KnowledgeLearningSkipped { reason } => Rendered {
            stdout: render_memory::learning_skipped_text(*reason),
            stderr: String::new(),
        },
        TerminalEvent::KnowledgeLearningDisabled { model, misses } => Rendered {
            stdout: render_memory::learning_disabled_text(model, *misses),
            stderr: String::new(),
        },
        TerminalEvent::ClarificationNeeded { question, options } => Rendered {
            stdout: render_clarification::clarification_text(question, options),
            stderr: String::new(),
        },
        TerminalEvent::Complete => Rendered {
            stdout: "\n".into(),
            stderr: String::new(),
        },
        // Text mode reports the scripted run's result only through the
        // unfinished-tools warning (render_turn); the result itself is a
        // machine event.
        TerminalEvent::TurnResult { .. } => Rendered {
            stdout: String::new(),
            stderr: String::new(),
        },
        TerminalEvent::TurnReset => Rendered {
            stdout: "provider stream interrupted — retrying\n".into(),
            stderr: String::new(),
        },
        TerminalEvent::AnswerDesignated { .. } => Rendered {
            stdout: String::new(),
            stderr: String::new(),
        },
        TerminalEvent::ConsensusDecided {
            sql,
            attempts,
            voted,
            votes,
            margin: _,
            tied,
            probe_broke_tie,
        } => Rendered {
            stdout: consensus_text(
                sql.as_deref(),
                *attempts,
                *voted,
                *votes,
                *tied,
                *probe_broke_tie,
            ),
            stderr: String::new(),
        },
        TerminalEvent::Usage { .. } => Rendered {
            stdout: String::new(),
            stderr: String::new(),
        },
        TerminalEvent::FailedAttemptUsage { .. } => Rendered {
            stdout: String::new(),
            stderr: String::new(),
        },
        TerminalEvent::ProviderRecovery {
            phase,
            reason,
            attempt,
            limit,
        } => Rendered {
            stdout: format!(
                "provider recovery: {} attempt {attempt} of {limit} ({})\n",
                recovery_phase(*phase),
                recovery_reason(*reason),
            ),
            stderr: String::new(),
        },
        TerminalEvent::Result { message } => Rendered {
            stdout: format!("{message}\n"),
            stderr: String::new(),
        },
        TerminalEvent::QueryResult { result } => Rendered {
            stdout: query_text(result),
            stderr: String::new(),
        },
        TerminalEvent::Schema { schema } => Rendered {
            stdout: format!("{}\n", schema_text(schema)),
            stderr: String::new(),
        },
        TerminalEvent::NotImplemented { feature } => Rendered {
            stdout: format!("Not implemented: {feature}\n"),
            stderr: String::new(),
        },
        TerminalEvent::DoctorTask { .. } => render_doctor::task_text(event),
        TerminalEvent::ContractList { contracts } => render_contract::list(contracts),
        TerminalEvent::ContractShow { contract } => render_contract::show(contract),
        TerminalEvent::ContractChanged {
            claim_id,
            action,
            status,
        } => render_contract::changed(claim_id, action, status),
        TerminalEvent::ContractRemembered {
            claim_id: _,
            object,
            kind,
            value,
            column,
            previous,
            action,
            status,
        } => render_contract::remembered(
            object,
            kind,
            value,
            column.as_deref(),
            previous.as_deref(),
            action,
            status,
        ),
        TerminalEvent::ContractQueue { items } => render_contract::queue(items),
        TerminalEvent::ContractImport { report } => render_io::import(report),
        TerminalEvent::ContractExport { report } => render_io::export(report),
    };
    Rendered {
        stdout: sanitize_terminal(&rendered.stdout),
        stderr: sanitize_terminal(&rendered.stderr),
    }
}
