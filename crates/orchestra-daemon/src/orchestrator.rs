//! Composing the team for a ticket.
//!
//! The orchestrator is itself a Claude Code run: read-only tools, a JSON schema
//! built from the role catalog, and a budget. It proposes; the user decides.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use orchestra_core::claude::StreamLine;
use orchestra_core::config::Config;
use orchestra_core::model::{
    Agent, AgentStatus, Effort, Project, Team, TeamProposal, Ticket, ORCHESTRATOR_ROLE,
};
use orchestra_core::roles::Catalog;
use orchestra_core::schema::{proposal_from_result, team_proposal_schema, MAX_TEAM};
use uuid::Uuid;

use crate::bus::EventBus;
use crate::ledger::UsageLedger;
use crate::repo_summary::RepoSummary;
use crate::worker::claude::{write_prompt_file, ClaudeCommand, ClaudeProcess, ProcessEvent};
use crate::worker::translate::{self, Scope};

/// The orchestrator's own instructions, shipped with the binary.
const ORCHESTRATOR_PROMPT: &str = include_str!("../../../assets/roles/_orchestrator.md");

/// What planning produced.
#[derive(Debug, Clone)]
pub struct Plan {
    pub proposal: TeamProposal,
    /// The proposal validated against the catalog, when it passes. A proposal
    /// that names an unknown role is still shown to the user, with the reason.
    pub team: Option<Team>,
    pub validation_error: Option<String>,
    pub session_id: Uuid,
}

/// Run the planning call for a ticket.
///
/// The tokens it spends are attributed to the ticket through a pseudo-agent, so
/// the cost of thinking about a feature is part of the feature's cost.
pub async fn plan(
    ticket: &Ticket,
    project: &Project,
    catalog: &Catalog,
    cfg: &Config,
    cache_dir: &Path,
    bus: &EventBus,
    ledger: &UsageLedger,
) -> Result<Plan> {
    anyhow::ensure!(
        !catalog.is_empty(),
        "aucun rôle disponible : lance « orchestra init » pour installer le catalogue"
    );

    let session_id = Uuid::new_v4();
    let agent = pseudo_agent(ticket, project, cfg, session_id);
    bus.store()
        .insert_agent(agent.clone())
        .await
        .context("enregistrement de l'agent orchestrateur")?;

    let names: Vec<String> = catalog.names().into_iter().collect();
    let schema = team_proposal_schema(&names);
    let prompt_file = write_prompt_file(cache_dir, "_orchestrator", ORCHESTRATOR_PROMPT)
        .context("écriture de la consigne de l'orchestrateur")?;

    // A tree walk and a few git calls: off the runtime, like all git.
    let repo = project.path.clone();
    let summary = crate::worktree::off_runtime(move || RepoSummary::build(&repo)).await;

    let mut cmd = ClaudeCommand::new(
        &cfg.daemon.claude_bin,
        &project.path,
        user_prompt(ticket, catalog, &project.path, &summary),
    );
    cmd.session_id = Some(session_id);
    cmd.name = Some(format!(
        "[{}] orchestrateur #{}",
        project.name, ticket.number
    ));
    cmd.model = cfg.orchestrator.model_flag().map(str::to_string);
    cmd.effort = Some(cfg.orchestrator.effort);
    // It reads to decide; it never writes, runs commands or browses.
    cmd.allowed_tools = vec!["Read".into(), "Glob".into(), "Grep".into()];
    cmd.disallowed_tools = vec![
        "Bash".into(),
        "Edit".into(),
        "Write".into(),
        "WebFetch".into(),
        "WebSearch".into(),
        "Task".into(),
    ];
    cmd.append_system_prompt_file = Some(prompt_file);
    cmd.json_schema = Some(schema.to_string());
    cmd.max_budget_usd = cfg.orchestrator.max_budget_usd;
    // Planning is a single question with a single answer.
    cmd.one_shot = true;

    let scope = Scope {
        agent_id: Some(agent.id),
        ticket_id: Some(ticket.id),
        project_id: Some(project.id),
        session_id,
    };

    let outcome = run(&cmd, &scope, bus, ledger, &agent).await;

    match outcome {
        Ok(result) => {
            let mut proposal =
                proposal_from_result(result.structured_output.as_ref(), result.text.as_deref())?;
            let known: BTreeSet<String> = catalog.names();
            // Every ticket ends on a relecture, whatever the orchestrator
            // thought of the change. It lands in the proposal the user reads
            // before accepting, so he can still take it out.
            if cfg.review.enabled && !known.contains(&cfg.review.role) {
                bus.warn(format!(
                    "relecture demandée mais le rôle « {} » est absent du catalogue : \
                     l'équipe partira sans relecteur",
                    cfg.review.role
                ))
                .await;
            }
            orchestra_core::review::append_reviewer(
                &mut proposal,
                &cfg.review,
                &known,
                &ticket.title,
            );
            let (team, validation_error) = match Team::from_proposal(&proposal, &known) {
                Ok(team) => (Some(team), None),
                Err(e) => (None, Some(e.to_string())),
            };
            finish_agent(bus, &agent, AgentStatus::Done, result.text.clone()).await;
            Ok(Plan {
                proposal,
                team,
                validation_error,
                session_id,
            })
        }
        Err(e) => {
            finish_agent(bus, &agent, AgentStatus::Failed, None).await;
            Err(e)
        }
    }
}

/// What the planning run returned.
struct RunResult {
    structured_output: Option<serde_json::Value>,
    text: Option<String>,
}

/// Drive the process, publishing its events, until it ends.
async fn run(
    cmd: &ClaudeCommand,
    scope: &Scope,
    bus: &EventBus,
    ledger: &UsageLedger,
    agent: &Agent,
) -> Result<RunResult> {
    let (mut process, mut rx) = ClaudeProcess::spawn(cmd).await?;
    let pid = process.pid();

    bus.publish(orchestra_core::events::NewEvent::for_agent(
        orchestra_core::events::EventKind::AgentSpawned {
            role: agent.role.clone(),
            session_id: agent.session_id,
            pid,
            cmdline: cmd.display(),
        },
        agent.project_id,
        agent.ticket_id,
        agent.id,
    ))
    .await?;

    let mut result: Option<RunResult> = None;
    let mut failure: Option<String> = None;
    let mut stderr_tail: Vec<String> = Vec::new();

    while let Some(event) = rx.recv().await {
        match event {
            ProcessEvent::Line(line) => {
                let mut finished = false;
                if let StreamLine::Result(r) = line.as_ref() {
                    finished = true;
                    if r.is_error {
                        failure = Some(
                            r.result
                                .clone()
                                .unwrap_or_else(|| format!("échec ({})", r.subtype)),
                        );
                    }
                    result = Some(RunResult {
                        structured_output: r.structured_output.clone(),
                        text: r.result.clone(),
                    });
                }
                for kind in translate::translate(scope, &line, orchestra_core::now()) {
                    // The run's own status is decided here, not by the stream.
                    if matches!(
                        kind,
                        orchestra_core::events::EventKind::AgentStatusChanged { .. }
                    ) {
                        continue;
                    }
                    // The stream is the live source of truth for what this run
                    // costs. The transcript watcher would find it too, seconds
                    // later, and the merge on message id makes the two agree.
                    if let orchestra_core::events::EventKind::Usage { sample } = &kind {
                        if let Err(e) = ledger.record((**sample).clone()).await {
                            tracing::warn!("coût non enregistré : {e:#}");
                        }
                    }
                    let _ = bus
                        .publish(orchestra_core::events::NewEvent::for_agent(
                            kind,
                            agent.project_id,
                            agent.ticket_id,
                            agent.id,
                        ))
                        .await;
                }
                if finished {
                    // The answer is in; do not wait on a process that might
                    // linger for another turn.
                    break;
                }
            }
            ProcessEvent::Unparsed { raw, error } => {
                bus.warn(format!(
                    "ligne du flux illisible ({error}) — format de Claude Code peut-être changé : {raw}"
                ))
                .await;
            }
            ProcessEvent::Stderr(line) => {
                if stderr_tail.len() == 10 {
                    stderr_tail.remove(0);
                }
                stderr_tail.push(line);
            }
        }
    }

    let status = process
        .stop(std::time::Duration::from_secs(5))
        .await
        .unwrap_or_else(|_| std::process::ExitStatus::default());
    match result {
        Some(r) if failure.is_none() => Ok(r),
        Some(_) => anyhow::bail!("l'orchestrateur a échoué : {}", failure.unwrap_or_default()),
        None => {
            let hint = if stderr_tail.is_empty() {
                String::new()
            } else {
                format!(" — {}", stderr_tail.join(" / "))
            };
            anyhow::bail!(
                "l'orchestrateur s'est arrêté sans réponse (code {}){hint}",
                status.code().unwrap_or(-1)
            )
        }
    }
}

/// The prompt: the brief, the catalog, and a map of the repository.
fn user_prompt(ticket: &Ticket, catalog: &Catalog, repo: &Path, summary: &RepoSummary) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Ticket #{} — {}\n\n",
        ticket.number, ticket.title
    ));
    out.push_str("## Brief\n\n");
    out.push_str(ticket.brief.trim());
    out.push_str("\n\n## Rôles disponibles\n\n");
    out.push_str(&catalog.summary_lines());
    out.push_str(&format!(
        "\n\nTu peux en choisir entre 1 et {MAX_TEAM}. Aucun autre rôle n'existe.\n"
    ));
    let adrs = crate::rules::adr_summary(repo);
    if !adrs.is_empty() {
        out.push_str(
            "\n## Décisions d'architecture du projet\n\n\
             Déjà prises : l'équipe les respecte, et leur texte complet est donné à \
             chaque agent.\n\n",
        );
        out.push_str(&adrs);
        out.push('\n');
    }
    if !summary.is_empty() {
        out.push_str("\n## Le dépôt\n\n");
        out.push_str(&summary.text);
    }
    out.push_str(
        "\nComposé l'équipe la plus petite qui livre ce ticket, et réponds selon le schéma.\n",
    );
    out
}

/// The row that carries the planning call's cost, so it lands on the ticket.
fn pseudo_agent(ticket: &Ticket, project: &Project, cfg: &Config, session_id: Uuid) -> Agent {
    Agent {
        id: Uuid::new_v4(),
        ticket_id: ticket.id,
        project_id: project.id,
        role: ORCHESTRATOR_ROLE.to_string(),
        objective: format!("composer l'équipe du ticket #{}", ticket.number),
        stage: 0,
        session_id,
        model: cfg
            .orchestrator
            .model_flag()
            .unwrap_or_default()
            .to_string(),
        effort: cfg.orchestrator.effort,
        max_budget_usd: cfg.orchestrator.max_budget_usd,
        status: AgentStatus::Starting,
        exit_reason: None,
        pid: None,
        pane_id: None,
        attempt: 1,
        handoff: None,
        started_at: Some(orchestra_core::now()),
        ended_at: None,
    }
}

async fn finish_agent(bus: &EventBus, agent: &Agent, status: AgentStatus, handoff: Option<String>) {
    let mut done = agent.clone();
    done.status = status;
    done.ended_at = Some(orchestra_core::now());
    done.handoff = handoff;
    if let Err(e) = bus.store().update_agent(done).await {
        tracing::warn!("mise à jour de l'orchestrateur impossible : {e}");
    }
}

/// Effort used when a role does not pick one.
pub fn effort_for(role_effort: Option<Effort>, default: Effort) -> Effort {
    role_effort.unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::{ProjectKind, TicketStatus};
    use std::path::PathBuf;

    fn ticket() -> Ticket {
        Ticket {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            number: 12,
            title: "Ajouter un cache".into(),
            brief: "Le rendu recalcule tout à chaque fois.\nOn veut un cache.".into(),
            status: TicketStatus::Draft,
            branch: None,
            worktree_path: None,
            proposal: None,
            team: None,
            created_at: orchestra_core::now(),
            updated_at: orchestra_core::now(),
        }
    }

    fn project(path: PathBuf) -> Project {
        Project {
            id: Uuid::new_v4(),
            name: "mon-projet".into(),
            path,
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        }
    }

    fn catalog(dir: &Path) -> Catalog {
        std::fs::write(
            dir.join("backend.md"),
            "---\nname: backend\ndescription: Logique serveur.\nmodel: sonnet\n---\nTu codes.\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("tests.md"),
            "---\nname: tests\ndescription: Couvre les cas limites.\n---\nTu testes.\n",
        )
        .unwrap();
        Catalog::load(dir, None)
    }

    #[test]
    fn the_prompt_carries_the_brief_the_catalog_and_the_repo() {
        let dir = tempfile::tempdir().unwrap();
        let roles = dir.path().join("roles");
        std::fs::create_dir_all(&roles).unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("README.md"), "# Mon dépôt").unwrap();

        let prompt = user_prompt(&ticket(), &catalog(&roles), &repo, &RepoSummary::build(&repo));
        assert!(prompt.contains("#12 — Ajouter un cache"));
        assert!(prompt.contains("On veut un cache"), "le brief entier");
        assert!(prompt.contains("- backend (sonnet) : Logique serveur."));
        assert!(prompt.contains("- tests (par défaut) :"));
        assert!(prompt.contains("Mon dépôt"), "le dépôt est résumé");
        assert!(
            prompt.contains("Aucun autre rôle n'existe"),
            "le catalogue est présenté comme fermé"
        );
    }

    #[test]
    fn the_command_line_is_read_only_and_schema_bound() {
        let dir = tempfile::tempdir().unwrap();
        let roles = dir.path().join("roles");
        std::fs::create_dir_all(&roles).unwrap();
        let cat = catalog(&roles);
        let cfg = Config::default();

        let mut cmd = ClaudeCommand::new("claude", dir.path(), "prompt");
        cmd.allowed_tools = vec!["Read".into(), "Glob".into(), "Grep".into()];
        cmd.disallowed_tools = vec!["Bash".into(), "Edit".into(), "Write".into()];
        cmd.json_schema =
            Some(team_proposal_schema(&cat.names().into_iter().collect::<Vec<_>>()).to_string());
        cmd.max_budget_usd = cfg.orchestrator.max_budget_usd;

        let args = cmd.args();
        assert!(args
            .windows(2)
            .any(|w| w == ["--allowedTools", "Read,Glob,Grep"]));
        assert!(args
            .iter()
            .any(|a| a.contains("\"enum\":[\"backend\",\"tests\"]")));
        assert!(args.windows(2).any(|w| w == ["--max-budget-usd", "2"]));
        // Nothing that could change the repository.
        let disallowed = args
            .windows(2)
            .find(|w| w[0] == "--disallowedTools")
            .map(|w| w[1].clone())
            .unwrap();
        for tool in ["Bash", "Edit", "Write"] {
            assert!(disallowed.contains(tool), "{tool} doit être interdit");
        }
    }

    #[test]
    fn the_pseudo_agent_attaches_the_cost_to_the_ticket() {
        let t = ticket();
        let p = project(PathBuf::from("/tmp"));
        let session = Uuid::new_v4();
        let agent = pseudo_agent(&t, &p, &Config::default(), session);
        assert_eq!(agent.role, ORCHESTRATOR_ROLE);
        assert_eq!(agent.ticket_id, t.id);
        assert_eq!(agent.project_id, p.id);
        assert_eq!(agent.session_id, session);
        assert_eq!(agent.stage, 0);
        assert!(agent.started_at.is_some());
    }

    #[tokio::test]
    async fn planning_without_a_catalog_says_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open_memory().unwrap();
        let bus = EventBus::new(store);
        let empty = Catalog::load(dir.path(), None);
        let ledger = UsageLedger::new(bus.store().clone(), &Config::default());
        let err = plan(
            &ticket(),
            &project(dir.path().to_path_buf()),
            &empty,
            &Config::default(),
            dir.path(),
            &bus,
            &ledger,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("orchestra init"), "{err}");
    }
}
