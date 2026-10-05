//! The daemon core: single owner of all mutable state.
//!
//! Connections never touch the state directly. They send a [`Command`] down an
//! mpsc channel with a oneshot to reply on, so there is exactly one writer and
//! no `Arc<Mutex<Daemon>>` anywhere.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use orchestra_core::config::{Config, ResolvedPaths};
use orchestra_core::events::{EventFilter, EventKind, NewEvent};
use orchestra_core::guard::GitPolicy;
use orchestra_core::conventions::{RuleKind, RuleStatus};
use orchestra_core::model::{
    can_transition, Agent, AgentStatus, Project, ProjectId, ProjectKind, RoleScope, Team, TeamProposal,
    Ticket,
    TicketId, TicketStatus, Todo, TodoId, TodoStatus,
};
use orchestra_core::protocol::{
    AgentSummary, ApiError, Command, DaemonStatus, Reply, TicketDetail, TicketSummary, UsageQuery,
    PROTOCOL_VERSION,
};
use orchestra_core::roles::Catalog;
use time::OffsetDateTime;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::bus::EventBus;
use crate::ledger::{AgentCost, UsageLedger};
use crate::store::Store;
use crate::supervisor::Supervisor;

/// A command plus where to send its reply.
pub struct Job {
    pub cmd: Command,
    pub reply: oneshot::Sender<Result<Reply, ApiError>>,
}

/// Handle used by connections to talk to the core.
#[derive(Clone)]
pub struct DaemonHandle {
    tx: mpsc::Sender<Job>,
    bus: EventBus,
}

impl DaemonHandle {
    pub fn bus(&self) -> &EventBus {
        &self.bus
    }

    pub async fn call(&self, cmd: Command) -> Result<Reply, ApiError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Job { cmd, reply: tx })
            .await
            .map_err(|_| ApiError::internal("le daemon ne répond plus"))?;
        rx.await
            .map_err(|_| ApiError::internal("le daemon a abandonné la commande"))?
    }
}

pub struct Daemon {
    supervisor: Supervisor,
    /// Tickets whose planning run is in flight, so a second request is refused
    /// instead of spending tokens twice.
    planning: HashSet<TicketId>,
    planning_done: mpsc::Sender<TicketId>,
    planning_rx: mpsc::Receiver<TicketId>,
    cfg: Arc<Config>,
    paths: ResolvedPaths,
    store: Store,
    ledger: UsageLedger,
    bus: EventBus,
    started_at: OffsetDateTime,
    rx: mpsc::Receiver<Job>,
    handle: DaemonHandle,
}

impl Daemon {
    pub fn new(cfg: Config, store: Store) -> Self {
        let paths = ResolvedPaths::from_config(&cfg);
        let bus = EventBus::new(store.clone());
        let (tx, rx) = mpsc::channel(256);
        let handle = DaemonHandle {
            tx,
            bus: bus.clone(),
        };
        let (planning_done, planning_rx) = mpsc::channel(16);
        let cfg = Arc::new(cfg);
        let ledger = UsageLedger::new(store.clone(), &cfg);
        let supervisor = Supervisor::new(
            store.clone(),
            bus.clone(),
            ledger.clone(),
            Arc::clone(&cfg),
            paths.cache_dir.clone(),
            paths.conventions_dir.clone(),
        );
        Daemon {
            supervisor,
            planning: HashSet::new(),
            planning_done,
            planning_rx,
            ledger,
            cfg,
            paths,
            store,
            bus,
            started_at: orchestra_core::now(),
            rx,
            handle,
        }
    }

    /// Agents die with the daemon that spawned them, so anything still marked
    /// as running on boot is a leftover from a crash or a kill. Left alone it
    /// would show as an eternally starting agent.
    pub async fn recover_on_boot(&self) -> Result<usize> {
        let stale = self
            .store
            .agents_with_status(vec![
                AgentStatus::Starting,
                AgentStatus::Running,
                AgentStatus::WaitingInput,
            ])
            .await?;
        let count = stale.len();
        for agent in stale {
            let mut recovered = agent.clone();
            recovered.status = AgentStatus::Crashed;
            recovered.exit_reason = Some(orchestra_core::model::ExitReason::Killed);
            recovered.ended_at = Some(orchestra_core::now());
            self.store.update_agent(recovered).await?;
            let _ = self
                .bus
                .publish(NewEvent::for_agent(
                    EventKind::AgentStatusChanged {
                        status: AgentStatus::Crashed,
                        reason: Some(orchestra_core::model::ExitReason::Killed),
                    },
                    agent.project_id,
                    agent.ticket_id,
                    agent.id,
                ))
                .await;
        }
        if count > 0 {
            tracing::info!(agents = count, "agents orphelins marqués comme arrêtés");
        }

        // A running ticket is a task walking its stages, and that task died
        // with the daemon. Left « en cours », it waits for a walker that will
        // never come back, and `launch` refuses it because it is already
        // running. Marked failed, it says what happened and can be relaunched
        // — the relaunch picks up where it stopped.
        let orphans = self
            .store
            .list_tickets(None, Some(vec![TicketStatus::Running]))
            .await?;
        for ticket in &orphans {
            let mut recovered = ticket.clone();
            recovered.status = TicketStatus::Failed;
            recovered.updated_at = orchestra_core::now();
            self.store.update_ticket(recovered).await?;
            let _ = self
                .bus
                .publish(
                    NewEvent::new(EventKind::TicketStatusChanged {
                        from: TicketStatus::Running,
                        to: TicketStatus::Failed,
                    })
                    .project(ticket.project_id)
                    .ticket(ticket.id),
                )
                .await;
            self.bus
                .warn(format!(
                    "le daemon a redémarré pendant le ticket #{} : relance-le, il reprendra \
                     où il s'est arrêté",
                    ticket.number
                ))
                .await;
        }
        if !orphans.is_empty() {
            tracing::info!(
                tickets = orphans.len(),
                "tickets orphelins rendus relançables"
            );
        }
        Ok(count)
    }

    pub fn handle(&self) -> DaemonHandle {
        self.handle.clone()
    }

    pub fn bus(&self) -> &EventBus {
        &self.bus
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn ledger(&self) -> &UsageLedger {
        &self.ledger
    }

    pub fn supervisor(&self) -> &Supervisor {
        &self.supervisor
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn paths(&self) -> &ResolvedPaths {
        &self.paths
    }

    /// Consume jobs until every handle is dropped.
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                job = self.rx.recv() => {
                    let Some(job) = job else { break };
                    let result = self.dispatch(job.cmd).await;
                    // A client that hung up mid-command is normal.
                    let _ = job.reply.send(result);
                }
                finished = self.planning_rx.recv() => {
                    if let Some(ticket_id) = finished {
                        self.planning.remove(&ticket_id);
                    }
                }
            }
        }
        tracing::info!("boucle du daemon terminée");
    }

    async fn dispatch(&mut self, cmd: Command) -> Result<Reply, ApiError> {
        match cmd {
            Command::Ping => Ok(Reply::Pong),
            Command::Status => self.status().await,
            Command::ListProjects => self.list_projects().await,
            Command::AddProject { path, name } => self.add_project(path, name).await,
            Command::ForgetProject { project_id } => {
                let project = self
                    .store
                    .project(project_id)
                    .await
                    .map_err(internal)?
                    .ok_or_else(|| ApiError::not_found("projet"))?;
                self.store
                    .delete_project(project_id)
                    .await
                    .map_err(|e| ApiError::invalid(e.to_string()))?;
                self.bus
                    .publish(
                        NewEvent::new(EventKind::ProjectForgotten {
                            name: project.name,
                        })
                        .project(project_id),
                    )
                    .await
                    .map_err(internal)?;
                Ok(Reply::Ack)
            }
            Command::ListTickets { project_id, status } => {
                self.list_tickets(project_id, status).await
            }
            Command::GetTicket { ticket_id } => self.get_ticket(ticket_id).await,
            Command::CreateTicket {
                project_id,
                title,
                brief,
            } => self.create_ticket(project_id, title, brief).await,
            Command::GetUsage { query } => self.get_usage(query).await,
            Command::PlanTicket { ticket_id } => self.plan_ticket(ticket_id).await,
            Command::AcceptProposal { ticket_id, team } => {
                self.accept_proposal(ticket_id, team).await
            }
            Command::ListRoles { project_id } => self.list_roles(project_id).await,
            Command::ListAgents { only_active } => self.list_agents(only_active).await,
            Command::LaunchTicket {
                ticket_id,
                open_panes,
            } => self.launch_ticket(ticket_id, open_panes).await,
            Command::CancelTicket { ticket_id } => {
                self.supervisor
                    .cancel_ticket(ticket_id)
                    .await
                    .map_err(internal)?;
                Ok(Reply::Ack)
            }
            Command::IntegrateTicket { ticket_id } => self.integrate_ticket(ticket_id).await,
            Command::FinishTicket { ticket_id } => self.finish_ticket(ticket_id).await,
            Command::ReopenTicket { ticket_id } => self.reopen_ticket(ticket_id).await,
            Command::SteerAgent {
                agent_id,
                text,
                hard,
            } => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Err(ApiError::invalid("la consigne est vide"));
                }
                self.supervisor
                    .steer(agent_id, text, hard)
                    .await
                    .map_err(|e| ApiError::conflict(e.to_string()))?;
                Ok(Reply::Ack)
            }
            Command::CancelAgent { agent_id } => {
                self.supervisor
                    .cancel_agent(agent_id)
                    .await
                    .map_err(|e| ApiError::conflict(e.to_string()))?;
                Ok(Reply::Ack)
            }
            // Subscribe is handled by the connection itself, never here.
            Command::Subscribe { .. } | Command::Unsubscribe => Err(ApiError::internal(
                "abonnement traité par la connexion, pas par le cœur",
            )),
            Command::OpenPane { agent_id } => self.open_pane(agent_id).await,
            Command::TakeOver { agent_id } => self.take_over(agent_id).await,
            Command::ListTodos => self.list_todos().await,
            Command::ListRules { project_id } => self.list_rules(project_id).await,
            Command::CreateRule {
                project_id,
                rule_kind,
                title,
            } => self.create_rule(project_id, rule_kind, title).await,
            Command::SetRuleStatus {
                project_id,
                rule_kind,
                name,
                status,
            } => self.set_rule_status(project_id, rule_kind, name, status).await,
            Command::DeleteRule {
                project_id,
                rule_kind,
                name,
            } => self.delete_rule(project_id, rule_kind, name).await,
            Command::PromoteRule { project_id, name } => self.promote_rule(project_id, name).await,
            Command::CreateRole { project_id, name } => self.create_role(project_id, name).await,
            Command::SetRoleGit {
                project_id,
                name,
                git,
            } => self.set_role_git(project_id, name, git).await,
            Command::DeleteRole { project_id, name } => self.delete_role(project_id, name).await,
            Command::PromoteRole { project_id, name } => self.promote_role(project_id, name).await,
            Command::CreateTodo {
                title,
                notes,
                urgent,
                due_at,
            } => self.create_todo(title, notes, urgent, due_at).await,
            Command::UpdateTodo {
                todo_id,
                title,
                notes,
                urgent,
                due_at,
            } => self.update_todo(todo_id, title, notes, urgent, due_at).await,
            Command::SetTodoStatus { todo_id, status } => {
                self.set_todo_status(todo_id, status).await
            }
            Command::DeleteTodo { todo_id } => self.delete_todo(todo_id).await,
            Command::PromoteTodo {
                todo_id,
                project_id,
                title,
                brief,
            } => self.promote_todo(todo_id, project_id, title, brief).await,
            // The guard decides on its own; nothing asks the daemon any more.
            Command::Hook { .. } => Ok(Reply::Hook {
                allow: true,
                reason: None,
            }),
        }
    }

    // -- handlers -----------------------------------------------------------

    async fn status(&self) -> Result<Reply, ApiError> {
        let projects = self.store.list_projects().await.map_err(internal)?;
        let running_tickets = self
            .store
            .list_tickets(None, Some(vec![TicketStatus::Running]))
            .await
            .map_err(internal)?;
        let last_seq = self.store.last_seq().await.map_err(internal)?;
        let watched_files = self.store.transcript_file_count().await.map_err(internal)?;
        Ok(Reply::Status {
            status: Box::new(DaemonStatus {
                version: orchestra_core::VERSION.to_string(),
                protocol: PROTOCOL_VERSION,
                started_at: self.started_at,
                projects: projects.len(),
                tickets_running: running_tickets.len(),
                agents_running: self.supervisor.running_agents().await,
                last_seq,
                watched_files,
                claude_version: None,
            }),
        })
    }

    async fn list_projects(&self) -> Result<Reply, ApiError> {
        let projects = self.store.list_projects().await.map_err(internal)?;
        Ok(Reply::Projects { projects })
    }

    async fn add_project(&self, path: PathBuf, name: Option<String>) -> Result<Reply, ApiError> {
        let path = canonical(&path)
            .map_err(|e| ApiError::invalid(format!("chemin inutilisable : {e}")))?;
        if !path.is_dir() {
            return Err(ApiError::invalid(format!(
                "{} n'est pas un dossier",
                path.display()
            )));
        }
        if let Some(existing) = self
            .store
            .project_by_path(path.clone())
            .await
            .map_err(internal)?
        {
            // Adopting a project we had only discovered is fine; a duplicate
            // managed one is not.
            if existing.kind == ProjectKind::Managed {
                return Err(ApiError::conflict(format!(
                    "le projet « {} » suit déjà ce chemin",
                    existing.name
                )));
            }
        }
        let name = name.unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string())
        });
        let probe = path.clone();
        let default_branch = crate::worktree::off_runtime(move || detect_default_branch(&probe))
            .await
            .unwrap_or_else(|| "main".into());
        let project = Project {
            id: Uuid::new_v4(),
            name: name.clone(),
            path: path.clone(),
            default_branch,
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        };
        self.store
            .insert_project(project.clone())
            .await
            .map_err(internal)?;
        self.bus
            .publish(
                NewEvent::new(EventKind::ProjectAdded {
                    name,
                    path: path.clone(),
                })
                .project(project.id),
            )
            .await
            .map_err(internal)?;
        Ok(Reply::Project {
            project: Box::new(project),
        })
    }

    async fn list_tickets(
        &self,
        project_id: Option<ProjectId>,
        status: Option<Vec<TicketStatus>>,
    ) -> Result<Reply, ApiError> {
        let tickets = self
            .store
            .list_tickets(project_id, status)
            .await
            .map_err(internal)?;
        let open_prs = crate::github::open_pull_requests(&self.store).await;
        // Agents and costs for the whole board in two reads. The board asks
        // every second, from every client; one pair of queries per card made
        // that 2N+2 round trips through the store's single connection.
        let mut agents: HashMap<TicketId, Vec<Agent>> = HashMap::new();
        for agent in self.store.agents_of_tickets(project_id).await.map_err(internal)? {
            agents.entry(agent.ticket_id).or_default().push(agent);
        }
        let mut costs = self.ledger.ticket_costs(project_id).await.map_err(internal)?;
        let stalled = self.supervisor.stalled_agents().await;
        let mut out = Vec::with_capacity(tickets.len());
        for ticket in tickets {
            let url = open_prs.get(&ticket.id).cloned();
            // Only a ticket « à relire » can be waiting on its merge, and those
            // are few: one query each is cheaper than reading every refusal.
            let blocked = if ticket.status == TicketStatus::Review {
                crate::integration::blocked_merge(&self.store, ticket.id).await
            } else {
                None
            };
            let team = agents.remove(&ticket.id).unwrap_or_default();
            // Verdict and checks matter only once the team handed back; those
            // tickets are few, so one read each stays cheap.
            let (verdict, checks_failed) = if ticket.status == TicketStatus::Review {
                let verdict = self.last_review(ticket.id).await?.map(|r| r.verdict);
                let failed = self
                    .last_checks(ticket.id)
                    .await?
                    .is_some_and(|c| c.failed().is_some());
                (verdict, failed)
            } else {
                (None, false)
            };
            let attention = orchestra_core::attention::of(&orchestra_core::attention::Facts {
                status: Some(ticket.status),
                has_proposal: ticket.proposal.is_some(),
                has_team: ticket.team.is_some(),
                agent_stalled: team.iter().any(|a| stalled.contains(&a.id)),
                verdict,
                checks_failed,
                merge_blocked: blocked.is_some(),
                pull_request_open: url.is_some(),
            });
            let cost = costs.remove(&ticket.id).unwrap_or_default();
            let mut summary = summarise(ticket, &team, cost);
            summary.pull_request = url;
            summary.merge_blocked = blocked.map(|b| b.reason);
            summary.attention = attention;
            out.push(summary);
        }
        out.sort_by_key(|s| (s.ticket.status.board_order(), -s.ticket.number));
        Ok(Reply::Tickets { tickets: out })
    }

    async fn summarise(&self, ticket: Ticket) -> Result<TicketSummary> {
        let agents = self.store.agents_of_ticket(ticket.id).await?;
        let cost = self.ledger.ticket_cost(ticket.id).await?;
        Ok(summarise(ticket, &agents, cost))
    }

    async fn get_ticket(&self, ticket_id: Uuid) -> Result<Reply, ApiError> {
        let ticket = self
            .store
            .ticket(ticket_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("ticket"))?;
        let project = self
            .store
            .project(ticket.project_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("projet du ticket"))?;
        let agents = self
            .store
            .agents_of_ticket(ticket_id)
            .await
            .map_err(internal)?;

        let mut costs = self.ledger.agent_costs(ticket_id).await.map_err(internal)?;
        let mut summaries = Vec::with_capacity(agents.len());
        for agent in agents {
            let cost = costs.remove(&agent.id).unwrap_or_default();
            summaries.push(AgentSummary {
                agent,
                tokens: cost.tokens,
                cost_usd: cost.cost_usd,
                turns: cost.messages as u32,
            });
        }
        // Read once for the whole ticket rather than summing the rows: the
        // ticket may also carry usage from an agent that no longer exists.
        let ticket_cost = self.ledger.ticket_cost(ticket_id).await.map_err(internal)?;
        let recent_events = self
            .store
            .recent_events(EventFilter::for_ticket(ticket_id), 50)
            .await
            .map_err(internal)?;

        let review = self.last_review(ticket_id).await?;
        let checks = self.last_checks(ticket_id).await?;
        let pull_request = crate::github::open_pull_request(&self.store, ticket_id).await;
        let merge_blocked = match ticket.status {
            TicketStatus::Review => crate::integration::blocked_merge(&self.store, ticket_id)
                .await
                .map(|b| b.reason),
            _ => None,
        };

        Ok(Reply::Ticket {
            detail: Box::new(TicketDetail {
                ticket,
                project,
                agents: summaries,
                tokens: ticket_cost.tokens,
                cost_usd: ticket_cost.cost_usd,
                recent_events,
                review,
                checks,
                pull_request,
                merge_blocked,
                integration_mode: self.cfg.integration.mode,
            }),
        })
    }

    async fn create_ticket(
        &self,
        project_id: ProjectId,
        title: String,
        brief: String,
    ) -> Result<Reply, ApiError> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(ApiError::invalid("le titre du ticket est vide"));
        }
        if self
            .store
            .project(project_id)
            .await
            .map_err(internal)?
            .is_none()
        {
            return Err(ApiError::not_found("projet"));
        }
        let number = self
            .store
            .next_ticket_number(project_id)
            .await
            .map_err(internal)?;
        let now = orchestra_core::now();
        let ticket = Ticket {
            id: Uuid::new_v4(),
            project_id,
            number,
            title: title.clone(),
            brief,
            status: TicketStatus::Draft,
            branch: None,
            worktree_path: None,
            proposal: None,
            team: None,
            created_at: now,
            updated_at: now,
        };
        self.store
            .insert_ticket(ticket.clone())
            .await
            .map_err(internal)?;
        self.bus
            .publish(
                NewEvent::new(EventKind::TicketCreated { number, title })
                    .project(project_id)
                    .ticket(ticket.id),
            )
            .await
            .map_err(internal)?;
        let summary = self.summarise(ticket).await.map_err(internal)?;
        Ok(Reply::Tickets {
            tickets: vec![summary],
        })
    }

    // -- todos ----------------------------------------------------------------

    async fn list_todos(&self) -> Result<Reply, ApiError> {
        let todos = self.store.list_todos().await.map_err(internal)?;
        Ok(Reply::Todos { todos })
    }

    async fn create_todo(
        &self,
        title: String,
        notes: String,
        urgent: bool,
        due_at: Option<OffsetDateTime>,
    ) -> Result<Reply, ApiError> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(ApiError::invalid("le titre du todo est vide"));
        }
        let now = orchestra_core::now();
        let todo = Todo {
            id: Uuid::new_v4(),
            title: title.clone(),
            notes,
            status: TodoStatus::Open,
            urgent,
            due_at,
            promoted_ticket_id: None,
            created_at: now,
            updated_at: now,
        };
        self.store
            .insert_todo(todo.clone())
            .await
            .map_err(internal)?;
        self.bus
            .publish(NewEvent::new(EventKind::TodoAdded { title }).todo(todo.id))
            .await
            .map_err(internal)?;
        Ok(Reply::Todos { todos: vec![todo] })
    }

    async fn update_todo(
        &self,
        todo_id: TodoId,
        title: String,
        notes: String,
        urgent: bool,
        due_at: Option<OffsetDateTime>,
    ) -> Result<Reply, ApiError> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(ApiError::invalid("le titre du todo est vide"));
        }
        let mut todo = self
            .store
            .todo(todo_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("todo"))?;
        todo.title = title.clone();
        todo.notes = notes;
        todo.urgent = urgent;
        todo.due_at = due_at;
        todo.updated_at = orchestra_core::now();
        self.store
            .update_todo(todo)
            .await
            .map_err(internal)?;
        self.bus
            .publish(NewEvent::new(EventKind::TodoUpdated { title }).todo(todo_id))
            .await
            .map_err(internal)?;
        Ok(Reply::Ack)
    }

    async fn set_todo_status(
        &self,
        todo_id: TodoId,
        status: TodoStatus,
    ) -> Result<Reply, ApiError> {
        let mut todo = self
            .store
            .todo(todo_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("todo"))?;
        let from = todo.status;
        todo.status = status;
        todo.updated_at = orchestra_core::now();
        let title = todo.title.clone();
        self.store
            .update_todo(todo)
            .await
            .map_err(internal)?;
        self.bus
            .publish(
                NewEvent::new(EventKind::TodoStatusChanged {
                    title,
                    from,
                    to: status,
                })
                .todo(todo_id),
            )
            .await
            .map_err(internal)?;
        Ok(Reply::Ack)
    }

    async fn delete_todo(&self, todo_id: TodoId) -> Result<Reply, ApiError> {
        let todo = self
            .store
            .todo(todo_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("todo"))?;
        self.store
            .delete_todo(todo_id)
            .await
            .map_err(|e| ApiError::invalid(e.to_string()))?;
        self.bus
            .publish(NewEvent::new(EventKind::TodoDeleted { title: todo.title }).todo(todo_id))
            .await
            .map_err(internal)?;
        Ok(Reply::Ack)
    }

    /// Creates a Draft ticket from this todo, then marks the todo promoted.
    /// The ticket's own creation is delegated to `create_ticket`, unchanged.
    async fn promote_todo(
        &self,
        todo_id: TodoId,
        project_id: ProjectId,
        title: String,
        brief: String,
    ) -> Result<Reply, ApiError> {
        let mut todo = self
            .store
            .todo(todo_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("todo"))?;
        if todo.promoted_ticket_id.is_some() {
            return Err(ApiError::conflict("ce todo est déjà promu en ticket"));
        }
        let project = self
            .store
            .project(project_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("projet"))?;
        let reply = self.create_ticket(project_id, title, brief).await?;
        let Reply::Tickets { tickets } = &reply else {
            return Err(ApiError::internal("création de ticket inattendue"));
        };
        let created = tickets
            .first()
            .ok_or_else(|| ApiError::internal("aucun ticket créé"))?;
        todo.status = TodoStatus::Done;
        todo.promoted_ticket_id = Some(created.ticket.id);
        todo.updated_at = orchestra_core::now();
        let title = todo.title.clone();
        let ticket_number = created.ticket.number;
        self.store
            .update_todo(todo)
            .await
            .map_err(internal)?;
        self.bus
            .publish(
                NewEvent::new(EventKind::TodoPromoted {
                    title,
                    ticket_number,
                    project_name: project.name,
                })
                .todo(todo_id)
                .project(project_id),
            )
            .await
            .map_err(internal)?;
        Ok(Reply::Ack)
    }

    // -- rules ----------------------------------------------------------------

    async fn project_opt(&self, id: Option<ProjectId>) -> Result<Option<Project>, ApiError> {
        match id {
            Some(id) => Ok(Some(
                self.store
                    .project(id)
                    .await
                    .map_err(internal)?
                    .ok_or_else(|| ApiError::not_found("projet"))?,
            )),
            None => Ok(None),
        }
    }

    fn rule_event(&self, project: Option<&Project>, kind: EventKind) -> NewEvent {
        let e = NewEvent::new(kind);
        match project {
            Some(p) => e.project(p.id),
            None => e,
        }
    }

    async fn list_rules(&self, project_id: Option<ProjectId>) -> Result<Reply, ApiError> {
        let project = self.project_opt(project_id).await?;
        let book = crate::rules::book(&self.paths.conventions_dir, project.as_ref());
        Ok(Reply::Rules {
            rules: book.rules,
            errors: book
                .errors
                .iter()
                .map(|(path, e)| format!("{} : {e}", path.display()))
                .collect(),
        })
    }

    /// A hand-made rule starts as a skeleton, so it starts `proposed`: the
    /// placeholder text must not reach an agent before it has been written.
    async fn create_rule(
        &self,
        project_id: Option<ProjectId>,
        kind: RuleKind,
        title: String,
    ) -> Result<Reply, ApiError> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(ApiError::invalid("la règle n'a pas de titre"));
        }
        let project = self.project_opt(project_id).await?;
        let draft = orchestra_core::conventions::RuleDraft {
            kind,
            title: &title,
            status: RuleStatus::Proposed,
            applies_to: &[],
            proposed_by: None,
            body: orchestra_core::conventions::skeleton_body(kind),
        };
        let (name, path) = crate::rules::create(&self.paths.conventions_dir, project.as_ref(), &draft)
            .map_err(|e| ApiError::invalid(format!("{e:#}")))?;
        self.bus
            .publish(self.rule_event(
                project.as_ref(),
                EventKind::RuleCreated {
                    rule_kind: kind,
                    name,
                    title,
                },
            ))
            .await
            .map_err(internal)?;
        Ok(Reply::RuleFile { path })
    }

    async fn set_rule_status(
        &self,
        project_id: Option<ProjectId>,
        kind: RuleKind,
        name: String,
        status: RuleStatus,
    ) -> Result<Reply, ApiError> {
        let project = self.project_opt(project_id).await?;
        let book = crate::rules::book(&self.paths.conventions_dir, project.as_ref());
        let (path, title, from) = crate::rules::source_of(&book, kind, &name)
            .map_err(|e| ApiError::invalid(format!("{e:#}")))?;
        if from == status {
            return Ok(Reply::Ack);
        }
        crate::rules::set_status(&path, status).map_err(|e| ApiError::invalid(format!("{e:#}")))?;
        self.bus
            .publish(self.rule_event(
                project.as_ref(),
                EventKind::RuleStatusChanged {
                    rule_kind: kind,
                    name,
                    title,
                    from,
                    to: status,
                },
            ))
            .await
            .map_err(internal)?;
        Ok(Reply::Ack)
    }

    async fn delete_rule(
        &self,
        project_id: Option<ProjectId>,
        kind: RuleKind,
        name: String,
    ) -> Result<Reply, ApiError> {
        let project = self.project_opt(project_id).await?;
        let book = crate::rules::book(&self.paths.conventions_dir, project.as_ref());
        let (path, title, _) = crate::rules::source_of(&book, kind, &name)
            .map_err(|e| ApiError::invalid(format!("{e:#}")))?;
        std::fs::remove_file(&path)
            .map_err(|e| ApiError::internal(format!("suppression de {} : {e}", path.display())))?;
        self.bus
            .publish(self.rule_event(
                project.as_ref(),
                EventKind::RuleDeleted {
                    rule_kind: kind,
                    name,
                    title,
                },
            ))
            .await
            .map_err(internal)?;
        Ok(Reply::Ack)
    }

    async fn promote_rule(&self, project_id: ProjectId, name: String) -> Result<Reply, ApiError> {
        let project = self
            .project_opt(Some(project_id))
            .await?
            .ok_or_else(|| ApiError::not_found("projet"))?;
        let (path, title) = crate::rules::promote(&self.paths.conventions_dir, &project, &name)
            .map_err(|e| ApiError::invalid(format!("{e:#}")))?;
        self.bus
            .publish(NewEvent::new(EventKind::RuleCreated {
                rule_kind: RuleKind::Convention,
                name,
                title,
            }))
            .await
            .map_err(internal)?;
        Ok(Reply::RuleFile { path })
    }

    /// Load the catalog a project sees: global roles plus its own overrides.
    fn catalog_for(&self, project: Option<&Project>) -> Catalog {
        crate::roles::catalog(&self.paths.roles_dir, project)
    }

    async fn list_roles(&self, project_id: Option<ProjectId>) -> Result<Reply, ApiError> {
        let project = match project_id {
            Some(id) => self.store.project(id).await.map_err(internal)?,
            None => None,
        };
        let mut catalog = self.catalog_for(project.as_ref());
        // Resolved here, where the config is: a client shows what an agent of
        // this role will really get, without knowing which role integrates.
        for role in &mut catalog.roles {
            role.git = Some(GitPolicy::for_role(
                role.git,
                &role.name,
                &self.cfg.integration.role,
            ));
        }
        // In the reply rather than as a warning: the roles are read every
        // second while their screen is open, and a broken file would flood
        // the activity with the same line.
        Ok(Reply::Roles {
            roles: catalog.roles,
            errors: catalog
                .errors
                .iter()
                .map(|(path, e)| format!("{} : {e}", path.display()))
                .collect(),
        })
    }

    async fn create_role(
        &self,
        project_id: Option<ProjectId>,
        name: String,
    ) -> Result<Reply, ApiError> {
        let project = self.project_opt(project_id).await?;
        let path = crate::roles::create(&self.paths.roles_dir, project.as_ref(), &name)
            .map_err(|e| ApiError::invalid(format!("{e:#}")))?;
        let scope = Some(if project.is_some() {
            RoleScope::Project
        } else {
            RoleScope::Global
        });
        self.bus
            .publish(self.rule_event(
                project.as_ref(),
                EventKind::RoleCreated {
                    name: name.trim().to_string(),
                    scope,
                },
            ))
            .await
            .map_err(internal)?;
        Ok(Reply::RoleFile { path })
    }

    /// What git a role gets is a line of its file, so it is changed where a
    /// human would change it — and read back by every launch after.
    async fn set_role_git(
        &self,
        project_id: Option<ProjectId>,
        name: String,
        git: GitPolicy,
    ) -> Result<Reply, ApiError> {
        let project = self.project_opt(project_id).await?;
        let catalog = self.catalog_for(project.as_ref());
        let role = crate::roles::find(&catalog, &name)
            .map_err(|e| ApiError::invalid(format!("{e:#}")))?;
        let before = GitPolicy::for_role(role.git, &role.name, &self.cfg.integration.role);
        if role.git == Some(git) {
            return Ok(Reply::Ack);
        }
        crate::roles::set_git(&role.source, git)
            .map_err(|e| ApiError::invalid(format!("{e:#}")))?;
        let change = if before == git {
            format!("{} désormais écrit dans son fichier", git.label_fr())
        } else {
            git.label_fr().to_string()
        };
        self.bus
            .publish(self.rule_event(project.as_ref(), EventKind::RoleUpdated { name, change }))
            .await
            .map_err(internal)?;
        Ok(Reply::Ack)
    }

    async fn delete_role(
        &self,
        project_id: Option<ProjectId>,
        name: String,
    ) -> Result<Reply, ApiError> {
        let project = self.project_opt(project_id).await?;
        let required = [
            self.cfg.review.role.as_str(),
            self.cfg.integration.role.as_str(),
        ];
        crate::roles::delete(&self.paths.roles_dir, project.as_ref(), &name, &required)
            .map_err(|e| ApiError::invalid(format!("{e:#}")))?;
        self.bus
            .publish(self.rule_event(project.as_ref(), EventKind::RoleDeleted { name }))
            .await
            .map_err(internal)?;
        Ok(Reply::Ack)
    }

    async fn promote_role(&self, project_id: ProjectId, name: String) -> Result<Reply, ApiError> {
        let project = self
            .project_opt(Some(project_id))
            .await?
            .ok_or_else(|| ApiError::not_found("projet"))?;
        let path = crate::roles::promote(&self.paths.roles_dir, &project, &name)
            .map_err(|e| ApiError::invalid(format!("{e:#}")))?;
        self.bus
            .publish(NewEvent::new(EventKind::RoleUpdated {
                name,
                change: "devenu global".into(),
            }))
            .await
            .map_err(internal)?;
        Ok(Reply::RoleFile { path })
    }

    /// Start the planning run. Replies at once; the proposal arrives as an
    /// event, because the call takes tens of seconds.
    async fn plan_ticket(&mut self, ticket_id: TicketId) -> Result<Reply, ApiError> {
        let ticket = self
            .store
            .ticket(ticket_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("ticket"))?;
        if !matches!(ticket.status, TicketStatus::Draft | TicketStatus::Planned) {
            return Err(ApiError::invalid(format!(
                "un ticket « {} » ne se planifie pas",
                ticket.status.label_fr()
            )));
        }
        if self.planning.contains(&ticket_id) {
            return Err(ApiError::conflict(
                "la planification de ce ticket est déjà en cours",
            ));
        }
        let project = self
            .store
            .project(ticket.project_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("projet du ticket"))?;
        let catalog = self.catalog_for(Some(&project));
        if catalog.is_empty() {
            return Err(ApiError::invalid(
                "aucun rôle disponible : lance « orchestra init » pour installer le catalogue",
            ));
        }

        self.planning.insert(ticket_id);
        let bus = self.bus.clone();
        let store = self.store.clone();
        let cfg = Arc::clone(&self.cfg);
        let cache_dir = self.paths.cache_dir.clone();
        let ledger = self.ledger.clone();
        let done = self.planning_done.clone();

        tokio::spawn(async move {
            let outcome = crate::orchestrator::plan(
                &ticket, &project, &catalog, &cfg, &cache_dir, &bus, &ledger,
            )
            .await;
            match outcome {
                Ok(plan) => {
                    let mut updated = ticket.clone();
                    updated.proposal = Some(plan.proposal.clone());
                    updated.updated_at = orchestra_core::now();
                    if let Err(e) = store.update_ticket(updated).await {
                        tracing::error!("proposition non enregistrée : {e:#}");
                    }
                    if let Some(reason) = &plan.validation_error {
                        bus.warn(format!("proposition à corriger : {reason}")).await;
                    }
                    let _ = bus
                        .publish(
                            NewEvent::new(EventKind::ProposalReady {
                                proposal: Box::new(plan.proposal),
                            })
                            .project(project.id)
                            .ticket(ticket.id),
                        )
                        .await;
                }
                Err(e) => {
                    let _ = bus
                        .publish(
                            NewEvent::new(EventKind::ProposalFailed {
                                error: format!("{e:#}"),
                            })
                            .project(project.id)
                            .ticket(ticket.id),
                        )
                        .await;
                }
            }
            let _ = done.send(ticket.id).await;
        });

        Ok(Reply::Ack)
    }

    /// Every agent, or only those still working.
    async fn list_agents(&self, only_active: bool) -> Result<Reply, ApiError> {
        let statuses = if only_active {
            vec![
                AgentStatus::Starting,
                AgentStatus::Running,
                AgentStatus::WaitingInput,
            ]
        } else {
            AgentStatus::ALL.to_vec()
        };
        let agents = self
            .store
            .agents_with_status(statuses)
            .await
            .map_err(internal)?;
        let mut out = Vec::with_capacity(agents.len());
        for agent in agents {
            let cost = self.ledger.agent_cost(agent.id).await.map_err(internal)?;
            out.push(AgentSummary {
                agent,
                tokens: cost.tokens,
                cost_usd: cost.cost_usd,
                turns: cost.messages as u32,
            });
        }
        Ok(Reply::Agents { agents: out })
    }

    /// Start the team of a planned ticket.
    async fn launch_ticket(
        &self,
        ticket_id: TicketId,
        open_panes: bool,
    ) -> Result<Reply, ApiError> {
        let ticket = self
            .store
            .ticket(ticket_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("ticket"))?;
        if ticket.team.is_none() {
            return Err(ApiError::invalid(
                "ce ticket n'a pas d'équipe acceptée : planifie-le puis accepte la proposition",
            ));
        }
        ensure_transition(ticket.status, TicketStatus::Running)?;
        let project = self
            .store
            .project(ticket.project_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("projet du ticket"))?;
        let catalog = self.catalog_for(Some(&project));

        self.supervisor
            .launch(ticket, project, catalog, open_panes)
            .await
            .map_err(|e| ApiError::conflict(format!("{e:#}")))?;
        Ok(Reply::Ack)
    }

    /// Run the integrator, then fuse the branch.
    ///
    /// Refused unless the last relecture said nothing blocks: the whole point
    /// of the role is that it comes after a green verdict, and a check made
    /// here holds for every client, not only for the screen that hides the key.
    async fn integrate_ticket(&self, ticket_id: TicketId) -> Result<Reply, ApiError> {
        let ticket = self
            .store
            .ticket(ticket_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("ticket"))?;
        if ticket.status != TicketStatus::Review {
            return Err(ApiError::invalid(format!(
                "ce ticket est « {} » : seul un ticket à relire s'intègre",
                ticket.status.label_fr()
            )));
        }
        match self.last_review(ticket_id).await? {
            Some(review) if review.is_ready() => {}
            Some(_) => {
                return Err(ApiError::invalid(
                    "la relecture bloque encore : corrige d'abord ce qu'elle a listé",
                ))
            }
            None => {
                return Err(ApiError::invalid(
                    "aucune relecture n'a rendu de verdict sur ce ticket",
                ))
            }
        }
        // A request already waiting for its human: reopening it would spend an
        // agent to land on the very same one.
        if let Some(url) = crate::github::open_pull_request(&self.store, ticket_id).await {
            return Err(ApiError::invalid(format!(
                "une pull request attend déjà sur ce ticket : {url}"
            )));
        }
        let project = self
            .store
            .project(ticket.project_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("projet du ticket"))?;
        let catalog = self.catalog_for(Some(&project));

        self.supervisor
            .integrate(ticket, project, catalog)
            .await
            .map_err(|e| ApiError::conflict(format!("{e:#}")))?;
        Ok(Reply::Ack)
    }

    /// Close a ticket the user merged himself.
    async fn finish_ticket(&self, ticket_id: TicketId) -> Result<Reply, ApiError> {
        let mut ticket = self
            .store
            .ticket(ticket_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("ticket"))?;
        ensure_transition(ticket.status, TicketStatus::Done)?;
        let from = ticket.status;
        ticket.status = TicketStatus::Done;
        ticket.updated_at = orchestra_core::now();
        self.store
            .update_ticket(ticket.clone())
            .await
            .map_err(internal)?;
        self.bus
            .publish(
                NewEvent::new(EventKind::TicketStatusChanged {
                    from,
                    to: TicketStatus::Done,
                })
                .project(ticket.project_id)
                .ticket(ticket.id),
            )
            .await
            .map_err(internal)?;
        Ok(Reply::Ack)
    }

    /// Send a closed ticket back to « à relire ».
    ///
    /// For the merge you want to redo another way — a pull request instead of a
    /// local fusion, say. The branch still holds the work and the verdict is
    /// still on record, so the ticket lands exactly where the decision was.
    async fn reopen_ticket(&self, ticket_id: TicketId) -> Result<Reply, ApiError> {
        let mut ticket = self
            .store
            .ticket(ticket_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("ticket"))?;
        // Only a closed ticket is reopened. A running one would be yanked out
        // from under its own team, and a failed one is relaunched instead —
        // that path already resumes where it stopped.
        match ticket.status {
            TicketStatus::Done | TicketStatus::Cancelled => {}
            TicketStatus::Failed => {
                return Err(ApiError::invalid(
                    "ce ticket a échoué : relance-le plutôt, il reprendra où il s'est arrêté",
                ))
            }
            other => {
                return Err(ApiError::invalid(format!(
                    "ce ticket est « {} » : seul un ticket fermé se rouvre",
                    other.label_fr()
                )))
            }
        }
        ensure_transition(ticket.status, TicketStatus::Review)?;
        let project = self
            .store
            .project(ticket.project_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("projet du ticket"))?;

        // Its checkout was cleaned up when it closed; the branch was not.
        self.supervisor
            .ensure_worktree(&mut ticket, &project)
            .await
            .map_err(|e| ApiError::conflict(format!("{e:#}")))?;

        let from = ticket.status;
        ticket.status = TicketStatus::Review;
        ticket.updated_at = orchestra_core::now();
        self.store
            .update_ticket(ticket.clone())
            .await
            .map_err(internal)?;
        self.bus
            .publish(
                NewEvent::new(EventKind::TicketStatusChanged {
                    from,
                    to: TicketStatus::Review,
                })
                .project(ticket.project_id)
                .ticket(ticket.id),
            )
            .await
            .map_err(internal)?;
        Ok(Reply::Ack)
    }

    /// The last verdict a relecture rendered on this ticket.
    async fn last_review(
        &self,
        ticket_id: TicketId,
    ) -> Result<Option<orchestra_core::protocol::ReviewOutcome>, ApiError> {
        let filter = EventFilter {
            ticket_id: Some(ticket_id),
            tags: vec![orchestra_core::events::EventTag::ReviewVerdict],
            ..Default::default()
        };
        let events = self
            .store
            .recent_events(filter, 1)
            .await
            .map_err(internal)?;
        Ok(events.into_iter().rev().find_map(|e| match e.kind {
            EventKind::ReviewVerdict {
                round,
                verdict,
                blocking,
                ..
            } => Some(orchestra_core::protocol::ReviewOutcome {
                round,
                verdict,
                blocking,
            }),
            _ => None,
        }))
    }

    /// What the repository's checks said on their last pass.
    ///
    /// Read from the ticket's whole history like the verdict, and for the same
    /// reason: this is what decides whether the branch can be integrated, so
    /// it must not depend on which events happen to be recent.
    async fn last_checks(
        &self,
        ticket_id: TicketId,
    ) -> Result<Option<orchestra_core::checks::ChecksOutcome>, ApiError> {
        let filter = EventFilter {
            ticket_id: Some(ticket_id),
            tags: vec![orchestra_core::events::EventTag::CheckFinished],
            ..Default::default()
        };
        // A pass is a handful of commands, and a ticket runs few passes.
        let events = self
            .store
            .recent_events(filter, 64)
            .await
            .map_err(internal)?;
        Ok(crate::checks::last_pass(&events))
    }

    /// Show the pane that follows an agent, opening one if there is none.
    ///
    /// Focusing an existing pane rather than opening a second one is the whole
    /// point of keeping its id: two panes tailing the same agent would show the
    /// same thing twice and hide something else.
    async fn open_pane(&self, agent_id: orchestra_core::model::AgentId) -> Result<Reply, ApiError> {
        if !crate::zellij::inside() {
            return Err(ApiError::unsupported(
                "pas de session zellij : lance le daemon depuis zellij pour avoir des panes",
            ));
        }
        let agent = self
            .store
            .agent(agent_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("agent"))?;

        if let Some(pane_id) = agent.pane_id.clone() {
            if crate::zellij::focus(&pane_id).await {
                return Ok(Reply::Pane { pane_id });
            }
            // The pane was closed by hand: forget it and open another.
        }
        let worktree = self.agent_worktree(&agent).await?;
        let pane_id = self
            .supervisor
            .open_tail_pane(agent, &worktree)
            .await
            .ok_or_else(|| ApiError::internal("zellij n'a pas ouvert le pane"))?;
        Ok(Reply::Pane { pane_id })
    }

    /// Hand an agent back to the user, in a session they drive themselves.
    ///
    /// The way out when an agent goes round in circles: its Claude session is
    /// still there, so `--resume` reopens the whole conversation in a pane. It
    /// stops being ours — `Manual` — but the watcher keeps counting it, since
    /// the session id has not changed.
    async fn take_over(&self, agent_id: orchestra_core::model::AgentId) -> Result<Reply, ApiError> {
        if !crate::zellij::inside() {
            return Err(ApiError::unsupported(
                "pas de session zellij : lance le daemon depuis zellij pour reprendre la main",
            ));
        }
        let agent = self
            .store
            .agent(agent_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("agent"))?;
        // Two hands on one session would each undo the other's turn.
        if agent.status.is_active() {
            return Err(ApiError::conflict(
                "cet agent tourne encore : arrête-le avant de reprendre la main",
            ));
        }
        let worktree = self.agent_worktree(&agent).await?;
        let pane_id = self
            .supervisor
            .hand_over(agent, &worktree, &self.cfg.daemon.claude_bin)
            .await
            .ok_or_else(|| ApiError::internal("zellij n'a pas ouvert le pane"))?;
        Ok(Reply::Pane { pane_id })
    }

    /// Where an agent worked, recreated from its branch if the folder is gone.
    async fn agent_worktree(
        &self,
        agent: &orchestra_core::model::Agent,
    ) -> Result<PathBuf, ApiError> {
        let ticket = self
            .store
            .ticket(agent.ticket_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("ticket de l'agent"))?;
        match ticket.worktree_path.filter(|p| p.exists()) {
            Some(path) => Ok(path),
            None => Err(ApiError::conflict(
                "le worktree de ce ticket n'existe plus : relance le ticket pour le recréer",
            )),
        }
    }

    /// Accept a team, possibly edited by the user, and move the ticket on.
    async fn accept_proposal(&self, ticket_id: TicketId, team: Team) -> Result<Reply, ApiError> {
        let mut ticket = self
            .store
            .ticket(ticket_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("ticket"))?;
        ensure_transition(ticket.status, TicketStatus::Planned)?;

        let project = self
            .store
            .project(ticket.project_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError::not_found("projet du ticket"))?;
        let catalog = self.catalog_for(Some(&project));

        // The user can edit the proposal, so it is validated again here rather
        // than trusted because the orchestrator produced it.
        let proposal = TeamProposal {
            summary: ticket
                .proposal
                .as_ref()
                .map(|p| p.summary.clone())
                .unwrap_or_default(),
            members: team.members.clone(),
            risks: ticket
                .proposal
                .as_ref()
                .map(|p| p.risks.clone())
                .unwrap_or_default(),
            estimated_size: ticket
                .proposal
                .as_ref()
                .map(|p| p.estimated_size)
                .unwrap_or(orchestra_core::model::Size::M),
        };
        let validated = Team::from_proposal(&proposal, &catalog.names())
            .map_err(|e| ApiError::invalid(e.to_string()))?;

        let from = ticket.status;
        ticket.team = Some(validated);
        ticket.status = TicketStatus::Planned;
        ticket.updated_at = orchestra_core::now();
        self.store
            .update_ticket(ticket.clone())
            .await
            .map_err(internal)?;
        self.bus
            .publish(
                NewEvent::new(EventKind::TicketStatusChanged {
                    from,
                    to: TicketStatus::Planned,
                })
                .project(ticket.project_id)
                .ticket(ticket.id),
            )
            .await
            .map_err(internal)?;

        self.get_ticket(ticket_id).await
    }

    async fn get_usage(&self, query: UsageQuery) -> Result<Reply, ApiError> {
        let (rows, totals) = self.ledger.rollup(query).await.map_err(internal)?;
        Ok(Reply::Usage { rows, totals })
    }
}

fn internal(e: anyhow::Error) -> ApiError {
    tracing::error!("erreur interne : {e:#}");
    ApiError::internal(e.to_string())
}

fn canonical(p: &Path) -> Result<PathBuf> {
    let expanded = if let Ok(stripped) = p.strip_prefix("~") {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME n'est pas défini")?;
        home.join(stripped)
    } else {
        p.to_path_buf()
    };
    std::fs::canonicalize(&expanded)
        .with_context(|| format!("résolution de {}", expanded.display()))
}

/// Ask git for the repository's default branch, falling back to `main`.
fn detect_default_branch(path: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C"])
        .arg(path)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}

/// Guard used by the ticket lifecycle once phases 2 and 3 land.
pub fn ensure_transition(from: TicketStatus, to: TicketStatus) -> Result<(), ApiError> {
    if can_transition(from, to) {
        Ok(())
    } else {
        Err(ApiError::invalid(format!(
            "un ticket « {} » ne peut pas passer à « {} »",
            from.label_fr(),
            to.label_fr()
        )))
    }
}

/// A board card: the ticket, how its team is doing, what it cost.
fn summarise(ticket: Ticket, agents: &[Agent], cost: AgentCost) -> TicketSummary {
    TicketSummary {
        agents_total: agents.len(),
        agents_active: agents.iter().filter(|a| a.status.is_active()).count(),
        agents_done: agents
            .iter()
            .filter(|a| a.status == AgentStatus::Done)
            .count(),
        cost_usd: cost.cost_usd,
        tokens: cost.tokens,
        pull_request: None,
        merge_blocked: None,
        attention: None,
        ticket,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::{Project, ProjectKind};
    use orchestra_core::review::Verdict;
    use uuid::Uuid;

    /// A daemon on an in-memory store, holding one ticket in the given state.
    async fn daemon_with_ticket(status: TicketStatus) -> (Daemon, Ticket) {
        let store = crate::store::Store::open_memory().unwrap();
        let project = Project {
            id: Uuid::new_v4(),
            name: "depot".into(),
            path: std::path::PathBuf::from("/tmp/depot"),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        };
        store.insert_project(project.clone()).await.unwrap();
        let ticket = Ticket {
            id: Uuid::new_v4(),
            project_id: project.id,
            number: 1,
            title: "cache".into(),
            brief: "b".into(),
            status,
            branch: Some("orch/1-cache".into()),
            worktree_path: Some(std::path::PathBuf::from("/tmp/wt/1-cache")),
            proposal: None,
            team: None,
            created_at: orchestra_core::now(),
            updated_at: orchestra_core::now(),
        };
        store.insert_ticket(ticket.clone()).await.unwrap();
        (Daemon::new(Config::default(), store), ticket)
    }

    #[tokio::test]
    async fn the_board_reads_in_bulk_what_it_used_to_read_card_by_card() {
        use orchestra_core::model::{Effort, Tokens, UsageSample, UsageSource};
        let (daemon, first) = daemon_with_ticket(TicketStatus::Running).await;
        let mut second = first.clone();
        second.id = Uuid::new_v4();
        second.number = 2;
        daemon.store.insert_ticket(second.clone()).await.unwrap();

        let agent = |ticket: &Ticket, role: &str, status: AgentStatus| Agent {
            id: Uuid::new_v4(),
            ticket_id: ticket.id,
            project_id: ticket.project_id,
            role: role.into(),
            objective: String::new(),
            stage: 0,
            session_id: Uuid::new_v4(),
            model: "sonnet".into(),
            effort: Effort::Medium,
            max_budget_usd: None,
            status,
            exit_reason: None,
            pid: None,
            pane_id: None,
            attempt: 1,
            handoff: None,
            started_at: None,
            ended_at: None,
        };
        let backend = agent(&first, "backend", AgentStatus::Running);
        let tests = agent(&first, "tests", AgentStatus::Done);
        let other = agent(&second, "docs", AgentStatus::Done);
        for a in [&backend, &tests, &other] {
            daemon.store.insert_agent(a.clone()).await.unwrap();
        }
        // Two models on one agent: each is priced on its own.
        let mut n = 0;
        for (a, model, out) in [
            (&backend, "claude-opus-5", 1_000),
            (&backend, "claude-haiku-4-5-20251001", 50_000),
            (&tests, "claude-sonnet-5", 7_000),
            (&other, "claude-sonnet-5", 300),
        ] {
            n += 1;
            daemon
                .ledger
                .record(UsageSample {
                    message_id: format!("msg_{n}"),
                    session_id: a.session_id,
                    subagent_id: None,
                    agent_id: Some(a.id),
                    ticket_id: Some(a.ticket_id),
                    project_id: Some(a.project_id),
                    model: model.into(),
                    tokens: Tokens { input: 10, output: out, ..Default::default() },
                    ts: orchestra_core::now(),
                    source: UsageSource::Stream,
                })
                .await
                .unwrap();
        }

        let Reply::Tickets { tickets } = daemon.list_tickets(None, None).await.unwrap() else {
            panic!("réponse inattendue");
        };
        for card in &tickets {
            let one = daemon.summarise(card.ticket.clone()).await.unwrap();
            assert_eq!(card.cost_usd, one.cost_usd, "#{}", card.ticket.number);
            assert_eq!(card.tokens, one.tokens);
            assert_eq!(
                (card.agents_total, card.agents_active, card.agents_done),
                (one.agents_total, one.agents_active, one.agents_done)
            );
        }
        let card = tickets.iter().find(|c| c.ticket.id == first.id).unwrap();
        assert_eq!((card.agents_total, card.agents_active, card.agents_done), (2, 1, 1));
        assert!(card.cost_usd.is_some());

        let Reply::Ticket { detail } = daemon.get_ticket(first.id).await.unwrap() else {
            panic!("réponse inattendue");
        };
        for row in &detail.agents {
            let one = daemon.ledger.agent_cost(row.agent.id).await.unwrap();
            assert_eq!(row.cost_usd, one.cost_usd, "{}", row.agent.role);
            assert_eq!(row.tokens, one.tokens);
            assert_eq!(row.turns, one.messages as u32);
        }
        assert_eq!(detail.agents.iter().find(|a| a.agent.id == backend.id).unwrap().turns, 2);
    }

    #[tokio::test]
    async fn the_board_says_what_each_ticket_waits_on_the_user_for() {
        use orchestra_core::attention::Attention;
        let attention = |reply: Reply| match reply {
            Reply::Tickets { tickets } => tickets[0].attention,
            other => panic!("réponse inattendue : {other:?}"),
        };

        let (daemon, ticket) = daemon_with_ticket(TicketStatus::Review).await;
        assert_eq!(
            attention(daemon.list_tickets(None, None).await.unwrap()),
            Some(Attention::ReviewBlocked),
            "rendu sans verdict : le silence ne vaut pas accord"
        );
        record_verdict(&daemon, &ticket, Verdict::Ready).await;
        assert_eq!(
            attention(daemon.list_tickets(None, None).await.unwrap()),
            Some(Attention::ReadyToIntegrate)
        );

        let (daemon, _) = daemon_with_ticket(TicketStatus::Running).await;
        assert_eq!(attention(daemon.list_tickets(None, None).await.unwrap()), None);
    }

    async fn record_verdict(daemon: &Daemon, ticket: &Ticket, verdict: Verdict) {
        daemon
            .bus
            .publish(
                NewEvent::new(EventKind::ReviewVerdict {
                    round: 1,
                    verdict,
                    blocking: vec![],
                    roles: vec![],
                })
                .project(ticket.project_id)
                .ticket(ticket.id),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn forgetting_a_project_publishes_the_event_the_ui_refreshes_on() {
        let store = crate::store::Store::open_memory().unwrap();
        let project = Project {
            id: Uuid::new_v4(),
            name: "depot".into(),
            path: std::path::PathBuf::from("/tmp/depot-oublie"),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        };
        let mut daemon = Daemon::new(Config::default(), store);
        daemon.store.insert_project(project.clone()).await.unwrap();

        daemon
            .dispatch(Command::ForgetProject {
                project_id: project.id,
            })
            .await
            .unwrap();

        assert!(daemon.store.project(project.id).await.unwrap().is_none());
        let events = daemon
            .store
            .recent_events(EventFilter::default(), 10)
            .await
            .unwrap();
        assert!(events.iter().any(|e| matches!(
            &e.kind,
            EventKind::ProjectForgotten { name } if name == "depot"
        )));
    }

    #[tokio::test]
    async fn forgetting_a_project_with_tickets_is_refused() {
        let (mut daemon, ticket) = daemon_with_ticket(TicketStatus::Draft).await;

        let err = daemon
            .dispatch(Command::ForgetProject {
                project_id: ticket.project_id,
            })
            .await
            .unwrap_err();

        assert!(err.message.contains("ticket"), "{}", err.message);
        assert!(daemon
            .store
            .project(ticket.project_id)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn a_ticket_orphaned_by_a_restart_becomes_relaunchable() {
        // La tâche qui déroulait les étapes meurt avec le daemon. Laissé « en
        // cours », le ticket attend un exécutant qui ne reviendra jamais, et
        // « lancer » le refuse puisqu'il tourne déjà.
        let (daemon, ticket) = daemon_with_ticket(TicketStatus::Running).await;
        daemon.recover_on_boot().await.unwrap();

        let back = daemon.store.ticket(ticket.id).await.unwrap().unwrap();
        assert_eq!(back.status, TicketStatus::Failed);
        assert!(
            orchestra_core::model::can_transition(back.status, TicketStatus::Running),
            "et il peut repartir"
        );

        // Un ticket déjà fini n'est pas touché.
        let (daemon, done) = daemon_with_ticket(TicketStatus::Review).await;
        daemon.recover_on_boot().await.unwrap();
        let back = daemon.store.ticket(done.id).await.unwrap().unwrap();
        assert_eq!(back.status, TicketStatus::Review);
    }

    #[tokio::test]
    async fn a_closed_ticket_can_be_sent_back_to_review() {
        // Le cas réel : fusionné en local, alors qu'on le voulait en pull
        // request. La branche porte toujours le travail.
        let (daemon, ticket) = daemon_with_ticket(TicketStatus::Done).await;
        // Sans dépôt git sous la main, la recréation du worktree échoue et
        // c'est elle qui parle — le statut, lui, n'a pas bougé.
        let err = daemon.reopen_ticket(ticket.id).await.unwrap_err();
        assert!(format!("{err:?}").contains("dépôt git"), "{err:?}");
        let back = daemon.store.ticket(ticket.id).await.unwrap().unwrap();
        assert_eq!(
            back.status,
            TicketStatus::Done,
            "rien n'a été changé à moitié"
        );

        // Et un ticket qui n'est pas fermé n'a rien à rouvrir.
        let (daemon, running) = daemon_with_ticket(TicketStatus::Running).await;
        let err = daemon.reopen_ticket(running.id).await.unwrap_err();
        assert!(
            format!("{err:?}").contains("seul un ticket fermé"),
            "{err:?}"
        );

        // Un ticket échoué se relance, il ne se rouvre pas.
        let (daemon, failed) = daemon_with_ticket(TicketStatus::Failed).await;
        let err = daemon.reopen_ticket(failed.id).await.unwrap_err();
        assert!(format!("{err:?}").contains("relance"), "{err:?}");
    }

    #[tokio::test]
    async fn a_ticket_whose_request_is_pending_is_not_reopened() {
        let (daemon, ticket) = daemon_with_ticket(TicketStatus::Review).await;
        record_verdict(&daemon, &ticket, Verdict::Ready).await;
        daemon
            .bus
            .publish(
                NewEvent::new(EventKind::PullRequestOpened {
                    url: "https://github.com/o/r/pull/12".into(),
                    number: Some(12),
                })
                .project(ticket.project_id)
                .ticket(ticket.id),
            )
            .await
            .unwrap();

        let err = daemon.integrate_ticket(ticket.id).await.unwrap_err();
        assert!(format!("{err:?}").contains("pull request"), "{err:?}");

        // Fermée, elle ne bloque plus rien.
        daemon
            .bus
            .publish(
                NewEvent::new(EventKind::PullRequestClosed {
                    url: "https://github.com/o/r/pull/12".into(),
                    merged: false,
                })
                .project(ticket.project_id)
                .ticket(ticket.id),
            )
            .await
            .unwrap();
        assert!(crate::github::open_pull_request(&daemon.store, ticket.id)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn a_ticket_still_running_is_not_integrated() {
        let (daemon, ticket) = daemon_with_ticket(TicketStatus::Running).await;
        record_verdict(&daemon, &ticket, Verdict::Ready).await;
        let err = daemon.integrate_ticket(ticket.id).await.unwrap_err();
        assert!(
            format!("{err:?}").contains("à relire"),
            "on dit pourquoi : {err:?}"
        );
    }

    #[tokio::test]
    async fn without_a_verdict_nothing_is_merged() {
        let (daemon, ticket) = daemon_with_ticket(TicketStatus::Review).await;
        let err = daemon.integrate_ticket(ticket.id).await.unwrap_err();
        assert!(format!("{err:?}").contains("relecture"), "{err:?}");
    }

    #[tokio::test]
    async fn a_blocking_verdict_stops_the_integration() {
        let (daemon, ticket) = daemon_with_ticket(TicketStatus::Review).await;
        record_verdict(&daemon, &ticket, Verdict::Changes).await;
        let err = daemon.integrate_ticket(ticket.id).await.unwrap_err();
        assert!(format!("{err:?}").contains("bloque"), "{err:?}");
    }

    #[tokio::test]
    async fn the_last_verdict_is_the_one_that_counts() {
        let (daemon, ticket) = daemon_with_ticket(TicketStatus::Review).await;
        record_verdict(&daemon, &ticket, Verdict::Changes).await;
        record_verdict(&daemon, &ticket, Verdict::Ready).await;
        let review = daemon.last_review(ticket.id).await.unwrap().unwrap();
        assert!(review.is_ready(), "le dernier tour fait foi");
    }

    #[tokio::test]
    async fn a_ticket_merged_by_hand_can_be_closed() {
        let (daemon, ticket) = daemon_with_ticket(TicketStatus::Review).await;
        daemon.finish_ticket(ticket.id).await.unwrap();
        let back = daemon.store.ticket(ticket.id).await.unwrap().unwrap();
        assert_eq!(back.status, TicketStatus::Done);

        // But a ticket that is still working is not closed behind its agents.
        let (daemon, ticket) = daemon_with_ticket(TicketStatus::Running).await;
        assert!(daemon.finish_ticket(ticket.id).await.is_err());
    }
}
