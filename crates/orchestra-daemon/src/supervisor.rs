//! Running a ticket's team.
//!
//! One task per ticket walks the stages in order, spawning one `claude` per
//! role in the ticket's worktree, streaming what it does, and handing its
//! closing summary to the next role. The user can queue a message for an agent
//! or interrupt it outright at any point.
//!
//! Sequential by design for now: several agents writing in one worktree would
//! conflict, and one process per role is what makes each of them visible and
//! steerable on its own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use orchestra_core::claude::StreamLine;
use orchestra_core::config::Config;
use orchestra_core::events::{EventKind, NewEvent};
use orchestra_core::model::{
    check_transition, Agent, AgentId, AgentStatus, Effort, ExitReason, Project, RoleDefinition,
    Team, Ticket, TicketId, TicketStatus,
};
use orchestra_core::roles::Catalog;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use crate::bus::EventBus;
use crate::hooks;
use crate::ledger::UsageLedger;
use crate::store::Store;
use crate::worker::claude::{write_prompt_file, ClaudeCommand, ClaudeProcess, ProcessEvent};
use crate::worker::translate::{self, Scope};
use crate::worktree;

/// Rules appended to every role's prompt.
const FOOTER: &str = include_str!("../../../assets/roles/_footer.md");

/// How long an interrupted agent is given to finish its turn cleanly.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// A message on its way to a running agent.
#[derive(Debug)]
enum Steer {
    /// Queued as a user turn; picked up after the current one.
    Queue(String),
    /// Interrupt now, then resume with this instruction.
    Redirect(String),
    Cancel,
}

/// Handle on one running agent.
struct Running {
    ticket_id: TicketId,
    steer: mpsc::Sender<Steer>,
}

/// Owns every running ticket.
#[derive(Clone)]
pub struct Supervisor {
    store: Store,
    bus: EventBus,
    ledger: UsageLedger,
    cfg: Arc<Config>,
    cache_dir: PathBuf,
    running: Arc<Mutex<HashMap<AgentId, Running>>>,
    tickets: Arc<Mutex<HashMap<TicketId, tokio::task::JoinHandle<()>>>>,
}

impl Supervisor {
    pub fn new(
        store: Store,
        bus: EventBus,
        ledger: UsageLedger,
        cfg: Arc<Config>,
        cache_dir: PathBuf,
    ) -> Self {
        Supervisor {
            store,
            bus,
            ledger,
            cfg,
            cache_dir,
            running: Arc::new(Mutex::new(HashMap::new())),
            tickets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn is_running(&self, ticket_id: TicketId) -> bool {
        self.tickets.lock().await.contains_key(&ticket_id)
    }

    pub async fn running_agents(&self) -> usize {
        self.running.lock().await.len()
    }

    /// Start the team of a planned ticket.
    pub async fn launch(&self, ticket: Ticket, project: Project, catalog: Catalog) -> Result<()> {
        let team = ticket
            .team
            .clone()
            .context("ce ticket n'a pas d'équipe acceptée")?;
        anyhow::ensure!(!self.is_running(ticket.id).await, "ce ticket tourne déjà");
        anyhow::ensure!(
            worktree::is_repository(&project.path),
            "{} n'est pas un dépôt git : un agent ne travaille que dans un worktree",
            project.path.display()
        );

        let plan = worktree::plan_for(
            &project,
            &ticket,
            &self
                .cfg
                .daemon
                .worktrees_dir
                .clone()
                .unwrap_or_else(orchestra_core::config::Paths::worktrees_dir),
            &self.cfg.daemon.branch_prefix,
        );
        let _ = worktree::prune(&project);
        let wt = worktree::ensure(&project, &plan)?;

        let mut ticket = ticket;
        let from = ticket.status;
        check_transition(from, TicketStatus::Running)?;
        ticket.branch = Some(wt.branch.clone());
        ticket.worktree_path = Some(wt.path.clone());
        ticket.status = TicketStatus::Running;
        ticket.updated_at = orchestra_core::now();
        self.store.update_ticket(ticket.clone()).await?;

        self.bus
            .publish(
                NewEvent::new(EventKind::WorktreeCreated {
                    path: wt.path.clone(),
                    branch: wt.branch.clone(),
                })
                .project(project.id)
                .ticket(ticket.id),
            )
            .await?;
        self.bus
            .publish(
                NewEvent::new(EventKind::TicketStatusChanged {
                    from,
                    to: TicketStatus::Running,
                })
                .project(project.id)
                .ticket(ticket.id),
            )
            .await?;

        let me = self.clone();
        let ticket_id = ticket.id;
        let handle = tokio::spawn(async move {
            let outcome = me.run_team(ticket, project, team, catalog, wt.path).await;
            if let Err(e) = outcome {
                tracing::error!("exécution du ticket interrompue : {e:#}");
            }
            me.tickets.lock().await.remove(&ticket_id);
        });
        self.tickets.lock().await.insert(ticket_id, handle);
        Ok(())
    }

    /// Walk the stages, one role at a time.
    async fn run_team(
        &self,
        ticket: Ticket,
        project: Project,
        team: Team,
        catalog: Catalog,
        worktree_path: PathBuf,
    ) -> Result<()> {
        let mut handoffs: Vec<(String, String)> = Vec::new();
        let mut failed = false;

        for (stage, member) in team.ordered() {
            let Some(role) = catalog.get(&member.role) else {
                self.warn_ticket(
                    ticket.id,
                    project.id,
                    format!(
                        "rôle « {} » absent du catalogue : étape ignorée",
                        member.role
                    ),
                )
                .await;
                failed = true;
                break;
            };

            let outcome = self
                .run_role(RoleRun {
                    ticket: &ticket,
                    project: &project,
                    role,
                    member,
                    stage,
                    worktree_path: &worktree_path,
                    handoffs: &handoffs,
                })
                .await;

            match outcome {
                Ok(AgentOutcome::Done { handoff }) => {
                    handoffs.push((member.role.clone(), handoff));
                }
                Ok(AgentOutcome::Cancelled) => {
                    self.finish_ticket(&ticket, TicketStatus::Cancelled).await;
                    return Ok(());
                }
                Ok(AgentOutcome::Failed { reason }) => {
                    self.bus
                        .warn(format!(
                            "l'agent « {} » s'est arrêté : {reason}",
                            member.role
                        ))
                        .await;
                    failed = true;
                    break;
                }
                Err(e) => {
                    self.bus
                        .warn(format!(
                            "l'agent « {} » n'a pas pu tourner : {e:#}",
                            member.role
                        ))
                        .await;
                    failed = true;
                    break;
                }
            }
        }

        let final_status = if failed {
            TicketStatus::Failed
        } else {
            TicketStatus::Review
        };
        self.finish_ticket(&ticket, final_status).await;
        Ok(())
    }

    /// Everything one role needs to run, grouped so the call stays readable.
    async fn run_role(&self, spec: RoleRun<'_>) -> Result<AgentOutcome> {
        let RoleRun {
            ticket,
            project,
            role,
            member,
            stage,
            worktree_path,
            handoffs,
        } = spec;
        let model = member
            .model
            .clone()
            .or_else(|| role.model.clone())
            .or_else(|| self.cfg.defaults.model_flag().map(str::to_string));
        let effort = member
            .effort
            .or(role.effort)
            .unwrap_or(self.cfg.defaults.effort);
        let budget = member
            .max_budget_usd
            .or(role.max_budget_usd)
            .or(self.cfg.defaults.max_budget_usd);

        let mut agent = Agent {
            id: Uuid::new_v4(),
            ticket_id: ticket.id,
            project_id: project.id,
            role: member.role.clone(),
            objective: member.objective.clone(),
            stage,
            session_id: Uuid::new_v4(),
            model: model.clone().unwrap_or_default(),
            effort,
            max_budget_usd: budget,
            status: AgentStatus::Pending,
            exit_reason: None,
            pid: None,
            pane_id: None,
            attempt: 1,
            handoff: None,
            started_at: None,
            ended_at: None,
        };
        self.store.insert_agent(agent.clone()).await?;

        let prompt = build_prompt(ticket, member, worktree_path, handoffs);
        let prompt_file = write_prompt_file(
            &self.cache_dir,
            &role.name,
            &format!("{}\n\n{FOOTER}", role.system_prompt),
        )?;

        loop {
            let resume = (agent.attempt > 1).then_some(agent.session_id);
            let turn_prompt = if resume.is_some() {
                // Restating the objective matters: told only that it was
                // interrupted, an agent concluded it had finished and
                // delivered nothing.
                resume_prompt(member, worktree_path)
            } else {
                prompt.clone()
            };

            let outcome = self
                .run_attempt(
                    &mut agent,
                    project,
                    role,
                    &prompt_file,
                    &turn_prompt,
                    worktree_path,
                    model.clone(),
                    effort,
                    budget,
                    resume,
                )
                .await?;

            match outcome {
                AgentOutcome::Failed { .. }
                    if agent.status == AgentStatus::Crashed
                        && agent.attempt < self.cfg.daemon.max_attempts =>
                {
                    agent.attempt += 1;
                    self.store.update_agent(agent.clone()).await?;
                    self.warn_ticket(
                        agent.ticket_id,
                        agent.project_id,
                        format!(
                            "l'agent « {} » a été relancé (tentative {})",
                            agent.role, agent.attempt
                        ),
                    )
                    .await;
                    continue;
                }
                other => return Ok(other),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_attempt(
        &self,
        agent: &mut Agent,
        project: &Project,
        role: &RoleDefinition,
        prompt_file: &Path,
        prompt: &str,
        worktree_path: &Path,
        model: Option<String>,
        effort: Effort,
        budget: Option<f64>,
        resume: Option<Uuid>,
    ) -> Result<AgentOutcome> {
        let mut cmd = ClaudeCommand::new(&self.cfg.daemon.claude_bin, worktree_path, prompt);
        cmd.session_id = Some(agent.session_id);
        cmd.resume = resume;
        cmd.name = Some(format!(
            "[{}] {} #{}",
            project.name,
            agent.role,
            ticket_number(agent)
        ));
        cmd.model = model;
        cmd.effort = Some(effort);
        cmd.permission_mode = Some("bypassPermissions".into());
        cmd.append_system_prompt_file = Some(prompt_file.to_path_buf());
        cmd.allowed_tools = role.allowed_tools.clone();
        cmd.disallowed_tools = role
            .disallowed_tools
            .iter()
            .cloned()
            .chain(hooks::always_disallowed())
            .collect();
        cmd.max_budget_usd = budget;
        cmd.settings_json = Some(hooks::settings_json(&hooks::hook_binary()));
        cmd.agents_json = role
            .subagents
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default());
        cmd.env = hooks::agent_env(
            worktree_path,
            &agent.id.to_string(),
            &agent.ticket_id.to_string(),
            &[],
        );

        let (mut process, mut rx) = ClaudeProcess::spawn(&cmd).await?;
        let (steer_tx, mut steer_rx) = mpsc::channel(8);

        agent.pid = Some(process.pid());
        agent.status = AgentStatus::Starting;
        agent.started_at.get_or_insert_with(orchestra_core::now);
        self.store.update_agent(agent.clone()).await?;
        self.running.lock().await.insert(
            agent.id,
            Running {
                ticket_id: agent.ticket_id,
                steer: steer_tx,
            },
        );
        self.emit(
            agent,
            EventKind::AgentSpawned {
                role: agent.role.clone(),
                session_id: agent.session_id,
                pid: process.pid(),
                cmdline: cmd.display(),
            },
        )
        .await;
        self.set_status(agent, AgentStatus::Starting, None).await;

        let scope = Scope {
            agent_id: Some(agent.id),
            ticket_id: Some(agent.ticket_id),
            project_id: Some(agent.project_id),
            session_id: agent.session_id,
        };

        let mut result_line: Option<(String, bool, Option<String>)> = None;
        let mut cancelled = false;
        let mut interrupted = false;
        let mut stderr_tail: Vec<String> = Vec::new();

        loop {
            tokio::select! {
                event = rx.recv() => {
                    let Some(event) = event else { break };
                    match event {
                        ProcessEvent::Line(line) => {
                            if let StreamLine::Result(r) = line.as_ref() {
                                result_line = Some((
                                    r.subtype.clone(),
                                    r.is_error,
                                    r.result.clone(),
                                ));
                            }
                            self.publish_line(agent, &scope, &line).await;
                            if result_line.is_some() {
                                break;
                            }
                        }
                        ProcessEvent::Unparsed { raw, error } => {
                            self.bus
                                .warn(format!(
                                    "ligne du flux illisible ({error}) : {raw}"
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
                steer = steer_rx.recv() => {
                    match steer {
                        Some(Steer::Queue(text)) => {
                            if let Err(e) = process.send_user(&text).await {
                                self.bus.warn(format!("consigne non transmise : {e}")).await;
                            } else {
                                self.emit(agent, EventKind::AgentSteered {
                                    text: translate::redact(&text),
                                    by: "user".into(),
                                    hard: false,
                                })
                                .await;
                            }
                        }
                        Some(Steer::Redirect(text)) => {
                            self.emit(agent, EventKind::AgentSteered {
                                text: translate::redact(&text),
                                by: "user".into(),
                                hard: true,
                            })
                            .await;
                            interrupted = true;
                            let _ = process.stop(STOP_GRACE).await;
                            // The redirection is delivered by the next attempt,
                            // which resumes the same session.
                            self.store.update_agent(agent.clone()).await?;
                            break;
                        }
                        Some(Steer::Cancel) => {
                            cancelled = true;
                            let _ = process.stop(STOP_GRACE).await;
                            break;
                        }
                        None => {}
                    }
                }
            }
        }

        self.running.lock().await.remove(&agent.id);
        // Closing stdin lets a finished process exit rather than wait.
        process.close_stdin();
        let status = process.stop(STOP_GRACE).await.ok();

        agent.ended_at = Some(orchestra_core::now());
        agent.pid = None;

        if cancelled {
            self.set_status(agent, AgentStatus::Cancelled, Some(ExitReason::Interrupted))
                .await;
            return Ok(AgentOutcome::Cancelled);
        }
        if interrupted {
            self.set_status(agent, AgentStatus::Crashed, Some(ExitReason::Interrupted))
                .await;
            return Ok(AgentOutcome::Failed {
                reason: "redirigé".into(),
            });
        }

        match result_line {
            Some((subtype, is_error, text)) => {
                let reason = translate::exit_reason(&subtype, is_error);
                let handoff = text.unwrap_or_default();
                agent.handoff = Some(handoff.clone());
                if reason.is_success() {
                    self.set_status(agent, AgentStatus::Done, Some(reason))
                        .await;
                    Ok(AgentOutcome::Done { handoff })
                } else {
                    let label = reason.label_fr();
                    self.set_status(agent, AgentStatus::Failed, Some(reason))
                        .await;
                    Ok(AgentOutcome::Failed { reason: label })
                }
            }
            None => {
                // No result line: the process died on its own.
                let code = status.and_then(|s| s.code()).unwrap_or(-1);
                self.set_status(agent, AgentStatus::Crashed, Some(ExitReason::Crashed(code)))
                    .await;
                let hint = if stderr_tail.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", stderr_tail.join(" / "))
                };
                Ok(AgentOutcome::Failed {
                    reason: format!("arrêté sans réponse (code {code}){hint}"),
                })
            }
        }
    }

    /// Turn one stream line into events, recording any cost it reports.
    async fn publish_line(&self, agent: &Agent, scope: &Scope, line: &StreamLine) {
        for kind in translate::translate(scope, line, orchestra_core::now()) {
            match &kind {
                // The supervisor owns the agent's status; the stream only
                // reports what the process said.
                EventKind::AgentStatusChanged { status, .. } => {
                    if *status == AgentStatus::Running {
                        let mut running = agent.clone();
                        running.status = AgentStatus::Running;
                        let _ = self.store.update_agent(running).await;
                        self.emit(
                            agent,
                            EventKind::AgentStatusChanged {
                                status: AgentStatus::Running,
                                reason: None,
                            },
                        )
                        .await;
                    }
                    continue;
                }
                EventKind::Usage { sample } => {
                    if let Err(e) = self.ledger.record((**sample).clone()).await {
                        tracing::warn!("coût non enregistré : {e:#}");
                    }
                }
                // A refusal by the guard is worth its own event: it is the
                // safety net doing its job, and the user should see it.
                EventKind::ToolFinished {
                    ok: false, summary, ..
                } => {
                    if let Some(reason) = hooks::blocked_reason(summary) {
                        self.emit(
                            agent,
                            EventKind::HookBlocked {
                                tool: "Bash".into(),
                                reason,
                            },
                        )
                        .await;
                    }
                }
                _ => {}
            }
            self.emit(agent, kind).await;
        }
    }

    /// A warning that belongs to a ticket, so it shows on that ticket rather
    /// than only in the global log.
    async fn warn_ticket(&self, ticket_id: TicketId, project_id: Uuid, message: String) {
        tracing::warn!("{message}");
        let _ = self
            .bus
            .publish(
                NewEvent::new(EventKind::Warning { message })
                    .project(project_id)
                    .ticket(ticket_id),
            )
            .await;
    }

    async fn emit(&self, agent: &Agent, kind: EventKind) {
        let _ = self
            .bus
            .publish(NewEvent::for_agent(
                kind,
                agent.project_id,
                agent.ticket_id,
                agent.id,
            ))
            .await;
    }

    async fn set_status(&self, agent: &mut Agent, status: AgentStatus, reason: Option<ExitReason>) {
        agent.status = status;
        agent.exit_reason = reason.clone();
        if let Err(e) = self.store.update_agent(agent.clone()).await {
            tracing::warn!("statut d'agent non enregistré : {e:#}");
        }
        self.emit(agent, EventKind::AgentStatusChanged { status, reason })
            .await;
    }

    async fn finish_ticket(&self, ticket: &Ticket, to: TicketStatus) {
        let Ok(Some(mut current)) = self.store.ticket(ticket.id).await else {
            return;
        };
        let from = current.status;
        if check_transition(from, to).is_err() {
            return;
        }
        current.status = to;
        current.updated_at = orchestra_core::now();
        if let Err(e) = self.store.update_ticket(current).await {
            tracing::warn!("statut de ticket non enregistré : {e:#}");
            return;
        }
        let _ = self
            .bus
            .publish(
                NewEvent::new(EventKind::TicketStatusChanged { from, to })
                    .project(ticket.project_id)
                    .ticket(ticket.id),
            )
            .await;
    }

    // -- steering ----------------------------------------------------------

    /// Queue a message for a running agent, or interrupt and resume with it.
    pub async fn steer(&self, agent_id: AgentId, text: String, hard: bool) -> Result<()> {
        let running = self.running.lock().await;
        let entry = running.get(&agent_id).context("cet agent ne tourne pas")?;
        let message = if hard {
            Steer::Redirect(text)
        } else {
            Steer::Queue(text)
        };
        entry
            .steer
            .send(message)
            .await
            .map_err(|_| anyhow::anyhow!("l'agent ne répond plus"))?;
        Ok(())
    }

    pub async fn cancel_agent(&self, agent_id: AgentId) -> Result<()> {
        let running = self.running.lock().await;
        let entry = running.get(&agent_id).context("cet agent ne tourne pas")?;
        entry
            .steer
            .send(Steer::Cancel)
            .await
            .map_err(|_| anyhow::anyhow!("l'agent ne répond plus"))?;
        Ok(())
    }

    /// Stop the whole ticket: its running agent first, then its task.
    pub async fn cancel_ticket(&self, ticket_id: TicketId) -> Result<()> {
        let agents: Vec<AgentId> = {
            let running = self.running.lock().await;
            running
                .iter()
                .filter(|(_, r)| r.ticket_id == ticket_id)
                .map(|(id, _)| *id)
                .collect()
        };
        for id in &agents {
            let _ = self.cancel_agent(*id).await;
        }
        if agents.is_empty() {
            // Nothing running: stop the walker and mark the ticket.
            if let Some(handle) = self.tickets.lock().await.remove(&ticket_id) {
                handle.abort();
            }
            if let Ok(Some(ticket)) = self.store.ticket(ticket_id).await {
                self.finish_ticket(&ticket, TicketStatus::Cancelled).await;
            }
        }
        Ok(())
    }
}

/// One role's run, as `run_team` hands it over.
struct RoleRun<'a> {
    ticket: &'a Ticket,
    project: &'a Project,
    role: &'a RoleDefinition,
    member: &'a orchestra_core::model::TeamMember,
    stage: u32,
    worktree_path: &'a Path,
    handoffs: &'a [(String, String)],
}

enum AgentOutcome {
    Done { handoff: String },
    Failed { reason: String },
    Cancelled,
}

fn ticket_number(agent: &Agent) -> String {
    // The number is not on the agent; the name is cosmetic, so the short id
    // is a good enough stand-in when it is not to hand.
    agent.ticket_id.to_string()[..8].to_string()
}

/// What a resumed agent is told.
///
/// Its session still holds the whole conversation, so the ticket is not
/// repeated, but the objective is: an agent told only "you were interrupted"
/// takes stock, decides it is done, and stops without delivering.
pub fn resume_prompt(member: &orchestra_core::model::TeamMember, worktree: &Path) -> String {
    format!(
        "Tu as été interrompu avant d'avoir fini. Reprends le travail.\n\n         Ton objectif reste : {}\n\n         Vérifie d'abord ce qui est déjà en place dans `{}` (fichiers et commits),          puis termine ce qui manque. Ne recommence pas ce qui est déjà fait, et ne          conclus pas que c'est terminé sans l'avoir constaté.",
        member.objective.trim(),
        worktree.display()
    )
}

/// The first message an agent receives: the ticket, its own objective, what
/// the team did before it, and the rules of the worktree.
pub fn build_prompt(
    ticket: &Ticket,
    member: &orchestra_core::model::TeamMember,
    worktree: &std::path::Path,
    handoffs: &[(String, String)],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Ticket #{} — {}\n\n",
        ticket.number, ticket.title
    ));
    out.push_str("## Brief\n\n");
    out.push_str(ticket.brief.trim());
    out.push_str(&format!(
        "\n\n## Ton objectif, en tant que « {} »\n\n{}\n",
        member.role,
        member.objective.trim()
    ));

    if !handoffs.is_empty() {
        out.push_str("\n## Ce que l'équipe a déjà fait\n");
        for (role, text) in handoffs {
            let capped = orchestra_core::claude::stream::truncate(text.trim(), 4000);
            out.push_str(&format!("\n### {role}\n\n{capped}\n"));
        }
    }

    out.push_str(&format!(
        "\n## Ton périmètre\n\nTu travailles dans `{}`. C'est un worktree git dédié à ce \
         ticket : tout ce que tu écris doit y rester.\n",
        worktree.display()
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::{ProjectKind, TeamMember};
    use std::path::Path;

    fn ticket() -> Ticket {
        Ticket {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            number: 7,
            title: "Ajouter un cache".into(),
            brief: "Le rendu recalcule tout.\nOn veut un cache.".into(),
            status: TicketStatus::Planned,
            branch: None,
            worktree_path: None,
            proposal: None,
            team: None,
            created_at: orchestra_core::now(),
            updated_at: orchestra_core::now(),
        }
    }

    fn member(role: &str) -> TeamMember {
        TeamMember {
            role: role.into(),
            objective: "écrire le cache dans store/rows.rs".into(),
            depends_on: vec![],
            model: None,
            effort: None,
            max_budget_usd: None,
            parallel_ok: false,
        }
    }

    #[test]
    fn the_prompt_carries_the_ticket_the_objective_and_the_boundary() {
        let prompt = build_prompt(
            &ticket(),
            &member("backend"),
            Path::new("/home/u/wt/7-cache"),
            &[],
        );
        assert!(prompt.contains("#7 — Ajouter un cache"));
        assert!(prompt.contains("On veut un cache"));
        assert!(prompt.contains("« backend »"));
        assert!(prompt.contains("store/rows.rs"));
        assert!(prompt.contains("/home/u/wt/7-cache"));
        assert!(prompt.contains("worktree git"));
        assert!(
            !prompt.contains("Ce que l'équipe a déjà fait"),
            "rien à transmettre au premier rôle"
        );
    }

    #[test]
    fn the_handoff_of_earlier_roles_is_passed_on_and_capped() {
        let long = "détail. ".repeat(2000);
        let handoffs = vec![
            (
                "architect".to_string(),
                "Plan écrit dans docs/.".to_string(),
            ),
            ("backend".to_string(), long),
        ];
        let prompt = build_prompt(
            &ticket(),
            &member("tests"),
            Path::new("/home/u/wt/7-cache"),
            &handoffs,
        );
        assert!(prompt.contains("### architect"));
        assert!(prompt.contains("Plan écrit dans docs/."));
        assert!(prompt.contains("### backend"));
        assert!(
            prompt.len() < 12_000,
            "un relais bavard ne doit pas noyer le prompt : {} caractères",
            prompt.len()
        );
    }

    #[test]
    fn a_resumed_agent_is_reminded_of_its_objective() {
        // Told only that it was interrupted, an agent concluded it had
        // finished and committed nothing.
        let prompt = resume_prompt(&member("backend"), Path::new("/home/u/wt/7-cache"));
        assert!(prompt.contains("interrompu"));
        assert!(
            prompt.contains("écrire le cache dans store/rows.rs"),
            "l'objectif doit être rappelé : {prompt}"
        );
        assert!(prompt.contains("/home/u/wt/7-cache"));
        assert!(
            prompt.contains("sans l'avoir constaté"),
            "et il doit vérifier avant de conclure"
        );
    }

    #[tokio::test]
    async fn a_ticket_without_a_team_is_refused() {
        let store = Store::open_memory().unwrap();
        let bus = EventBus::new(store.clone());
        let cfg = Arc::new(Config::default());
        let ledger = UsageLedger::new(store.clone(), &cfg);
        let dir = tempfile::tempdir().unwrap();
        let sup = Supervisor::new(store, bus, ledger, cfg, dir.path().to_path_buf());

        let project = Project {
            id: Uuid::new_v4(),
            name: "p".into(),
            path: dir.path().to_path_buf(),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        };
        let catalog = Catalog::load(dir.path(), None);
        let err = sup.launch(ticket(), project, catalog).await.unwrap_err();
        assert!(err.to_string().contains("équipe acceptée"), "{err}");
    }

    #[tokio::test]
    async fn steering_an_agent_that_is_not_running_says_so() {
        let store = Store::open_memory().unwrap();
        let bus = EventBus::new(store.clone());
        let cfg = Arc::new(Config::default());
        let ledger = UsageLedger::new(store.clone(), &cfg);
        let dir = tempfile::tempdir().unwrap();
        let sup = Supervisor::new(store, bus, ledger, cfg, dir.path().to_path_buf());

        let err = sup
            .steer(Uuid::new_v4(), "vas-y".into(), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("ne tourne pas"), "{err}");
        assert!(sup.cancel_agent(Uuid::new_v4()).await.is_err());
        // Cancelling an unknown ticket is harmless.
        assert!(sup.cancel_ticket(Uuid::new_v4()).await.is_ok());
    }
}
