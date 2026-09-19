//! The daemon core: single owner of all mutable state.
//!
//! Connections never touch the state directly. They send a [`Command`] down an
//! mpsc channel with a oneshot to reply on, so there is exactly one writer and
//! no `Arc<Mutex<Daemon>>` anywhere.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use orchestra_core::config::{Config, Paths, ResolvedPaths};
use orchestra_core::events::{EventFilter, EventKind, NewEvent};
use orchestra_core::model::{
    can_transition, AgentStatus, Project, ProjectId, ProjectKind, Team, TeamProposal, Ticket,
    TicketId, TicketStatus,
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
use crate::ledger::UsageLedger;
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
                    .warn(format!("projet « {} » oublié", project.name))
                    .await;
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
            Command::LaunchTicket { ticket_id, .. } => self.launch_ticket(ticket_id).await,
            Command::CancelTicket { ticket_id } => {
                self.supervisor
                    .cancel_ticket(ticket_id)
                    .await
                    .map_err(internal)?;
                Ok(Reply::Ack)
            }
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
            // Phase 4: zellij panes.
            Command::OpenPane { .. } => Err(ApiError::unsupported(
                "les panes zellij arrivent en phase 4",
            )),
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
        let default_branch = detect_default_branch(&path).unwrap_or_else(|| "main".into());
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
        let mut out = Vec::with_capacity(tickets.len());
        for ticket in tickets {
            out.push(self.summarise(ticket).await.map_err(internal)?);
        }
        out.sort_by_key(|s| (s.ticket.status.board_order(), -s.ticket.number));
        Ok(Reply::Tickets { tickets: out })
    }

    async fn summarise(&self, ticket: Ticket) -> Result<TicketSummary> {
        let agents = self.store.agents_of_ticket(ticket.id).await?;
        let cost = self.ledger.ticket_cost(ticket.id).await?;
        Ok(TicketSummary {
            agents_total: agents.len(),
            agents_active: agents.iter().filter(|a| a.status.is_active()).count(),
            agents_done: agents
                .iter()
                .filter(|a| a.status == AgentStatus::Done)
                .count(),
            cost_usd: cost.cost_usd,
            tokens: cost.tokens,
            ticket,
        })
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

        let mut summaries = Vec::with_capacity(agents.len());
        for agent in agents {
            let cost = self.ledger.agent_cost(agent.id).await.map_err(internal)?;
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

        Ok(Reply::Ticket {
            detail: Box::new(TicketDetail {
                ticket,
                project,
                agents: summaries,
                tokens: ticket_cost.tokens,
                cost_usd: ticket_cost.cost_usd,
                recent_events,
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

    /// Load the catalog a project sees: global roles plus its own overrides.
    fn catalog_for(&self, project: Option<&Project>) -> Catalog {
        let project_dir = project.map(|p| Paths::project_roles_dir(&p.path));
        Catalog::load(&self.paths.roles_dir, project_dir.as_deref())
    }

    async fn list_roles(&self, project_id: Option<ProjectId>) -> Result<Reply, ApiError> {
        let project = match project_id {
            Some(id) => self.store.project(id).await.map_err(internal)?,
            None => None,
        };
        let catalog = self.catalog_for(project.as_ref());
        for (path, err) in &catalog.errors {
            self.bus
                .warn(format!("rôle illisible {} : {err}", path.display()))
                .await;
        }
        Ok(Reply::Roles {
            roles: catalog.roles,
        })
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
    async fn launch_ticket(&self, ticket_id: TicketId) -> Result<Reply, ApiError> {
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
            .launch(ticket, project, catalog)
            .await
            .map_err(|e| ApiError::conflict(format!("{e:#}")))?;
        Ok(Reply::Ack)
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
