use super::*;

pub(super) async fn run_investigation_outcome_controlled_in(
    repo: &InvestigationRepository,
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
    control: RunControl<'_>,
) -> Result<RunOutcome, Box<dyn std::error::Error>> {
    match command {
        InvestigationCommand::Save {
            name,
            description,
            sql,
            file,
            connection,
            param_specs,
        } => save::save(
            repo,
            runtime,
            format,
            save_input::SaveRequest {
                name: &name,
                description: description.as_deref(),
                sql,
                file,
                connection: connection.as_deref(),
                param_specs: &param_specs,
            },
        )
        .map(RunOutcome::plain),
        InvestigationCommand::Edit {
            id,
            name,
            description,
            sql,
            file,
            param_specs,
        } => edit::edit(
            repo,
            format,
            edit::EditRequest {
                id: &id,
                name: name.as_deref(),
                description: description.as_deref(),
                sql,
                file,
                param_specs: &param_specs,
            },
        )
        .map(RunOutcome::plain),
        InvestigationCommand::List { limit, offset } => {
            list::list(repo, format, limit, offset).map(RunOutcome::plain)
        }
        InvestigationCommand::Show { id } => show::show(repo, format, &id).map(RunOutcome::plain),
        InvestigationCommand::Delete { id, revision } => {
            delete::delete(repo, format, &id, revision).map(RunOutcome::plain)
        }
        InvestigationCommand::Export {
            id,
            path,
            overwrite,
        } => export::export(repo, format, &id, &path, overwrite).map(RunOutcome::plain),
        InvestigationCommand::Import { path } => {
            import::import(repo, format, &path).map(RunOutcome::plain)
        }
        InvestigationCommand::Run {
            id,
            connection,
            revalidate,
            report,
            rows,
            overwrite,
            params,
        } => {
            run::run(
                repo,
                runtime,
                format,
                can_prompt,
                state_db,
                run::RunRequest {
                    id: &id,
                    connection: connection.as_deref(),
                    revalidate,
                    report: report.as_deref(),
                    rows,
                    overwrite,
                    params,
                    machine_driven: control.machine_driven,
                    cancellation: control.cancellation,
                },
            )
            .await
        }
    }
}

impl InvestigationCommand {
    /// The machine-driven seam (FIX-S6a), used only by the MCP replay
    /// adapter: the run can never write or complete review authority — an
    /// unreviewed investigation refuses; a reviewed one never refreshes.
    pub(crate) async fn run_outcome_machine_driven_with_cancellation(
        self,
        repo: &InvestigationRepository,
        runtime: &RuntimeConfig,
        format: RenderFormat,
        can_prompt: bool,
        state_db: &SqliteStateStore,
        cancellation: &CancellationToken,
    ) -> Result<RunOutcome, Box<dyn std::error::Error>> {
        run_investigation_outcome_controlled_in(
            repo,
            self,
            runtime,
            format,
            can_prompt,
            state_db,
            RunControl {
                cancellation: Some(cancellation),
                machine_driven: true,
            },
        )
        .await
    }
}
