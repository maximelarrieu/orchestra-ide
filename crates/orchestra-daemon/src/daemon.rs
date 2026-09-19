//! The daemon core: single owner of all mutable state.
//!
//! Connections never touch the state directly. They send a [`Command`] down an
//! mpsc channel with a oneshot to reply on, so there is exactly one writer and
//! no `Arc<Mutex<Daemon>>` anywhere.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use orchestra_core::config::{Config, ResolvedPaths};
use orchestra_core::events::{EventFilter, EventKind, NewEvent};
use orchestra_core::model::{
    can_transition, AgentStatus, Project, ProjectId, ProjectKind, Ticket, TicketStatus, Tokens,
};
use orchestra_core::protocol::{
    AgentSummary, ApiError, Command, DaemonStatus, Reply, TicketDetail, TicketSummary, UsageQuery,
    UsageRow, UsageTotals, PROTOCOL_VERSION,
};
use time::OffsetDateTime;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::bus::EventBus;
use crate::store::Store;

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
    cfg: Arc<Config>,
    paths: ResolvedPaths,
    store: Store,
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
        Daemon {
            cfg: Arc::new(cfg),
            paths,
            store,
            bus,
            started_at: orchestra_core::now(),
            rx,
            handle,
        }
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

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn paths(&self) -> &ResolvedPaths {
        &self.paths
    }

    /// Consume jobs until every handle is dropped.
    pub async fn run(mut self) {
        while let Some(job) = self.rx.recv().await {
            let result = self.dispatch(job.cmd).await;
            // A client that hung up mid-command is normal.
            let _ = job.reply.send(result);
        }
        tracing::info!("boucle du daemon terminée");
    }

    async fn dispatch(&mut self, cmd: Command) -> Result<Reply, ApiError> {
        match cmd {
            Command::Ping => Ok(Reply::Pong),
            Command::Status => self.status().await,
            Command::ListProjects => self.list_projects().await,
            Command::AddProject { path, name } => self.add_project(path, name).await,
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
            // Subscribe is handled by the connection itself, never here.
            Command::Subscribe { .. } | Command::Unsubscribe => Err(ApiError::internal(
                "abonnement traité par la connexion, pas par le cœur",
            )),
            // Phases 2 and 3.
            Command::PlanTicket { .. }
            | Command::AcceptProposal { .. }
            | Command::LaunchTicket { .. }
            | Command::CancelTicket { .. }
            | Command::SteerAgent { .. }
            | Command::CancelAgent { .. }
            | Command::OpenPane { .. }
            | Command::ListRoles { .. }
            | Command::Hook { .. } => Err(ApiError::unsupported(
                "commande pas encore disponible (phases 2 et 3)",
            )),
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
        let running_agents = self
            .store
            .agents_with_status(vec![
                AgentStatus::Starting,
                AgentStatus::Running,
                AgentStatus::WaitingInput,
            ])
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
                agents_running: running_agents.len(),
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
        let tokens = self.store.ticket_usage(ticket.id).await?;
        Ok(TicketSummary {
            agents_total: agents.len(),
            agents_active: agents.iter().filter(|a| a.status.is_active()).count(),
            agents_done: agents
                .iter()
                .filter(|a| a.status == AgentStatus::Done)
                .count(),
            cost_usd: self.cfg.pricing.cost(&dominant_model(&agents), &tokens),
            tokens,
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
        let mut tokens = Tokens::default();
        let mut cost = 0.0;
        let mut priced = false;
        for agent in agents {
            let (t, turns) = self.store.agent_usage(agent.id).await.map_err(internal)?;
            tokens += t;
            let c = self.cfg.pricing.cost(&agent.model, &t);
            if let Some(c) = c {
                cost += c;
                priced = true;
            }
            summaries.push(AgentSummary {
                agent,
                tokens: t,
                cost_usd: c,
                turns,
            });
        }
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
                tokens,
                cost_usd: priced.then_some(cost),
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

    async fn get_usage(&self, query: UsageQuery) -> Result<Reply, ApiError> {
        let (rows, totals) = self.store.usage_rollup(query).await.map_err(internal)?;
        let (rows, totals) = self.price(rows, totals);
        Ok(Reply::Usage { rows, totals })
    }

    /// Apply the price table. Rows carry the models they aggregate, so a row
    /// spanning several models is priced per model, not by a single guess.
    fn price(&self, rows: Vec<UsageRow>, mut totals: UsageTotals) -> (Vec<UsageRow>, UsageTotals) {
        let table = &self.cfg.pricing;
        let mut total_cost = 0.0;
        let mut any = false;
        let rows = rows
            .into_iter()
            .map(|mut r| {
                let models = r.keys.remove("models").unwrap_or_default();
                let list: Vec<&str> = models.split(',').filter(|s| !s.is_empty()).collect();
                // One model: price exactly. Several: price the row's totals with
                // each model's rate weighted evenly, which is the best we can do
                // without a per-model breakdown of this row.
                let cost = if list.len() <= 1 {
                    let m = list.first().copied().unwrap_or("");
                    table.cost(m, &r.tokens)
                } else {
                    let costs: Vec<f64> = list
                        .iter()
                        .filter_map(|m| table.cost(m, &r.tokens))
                        .collect();
                    if costs.is_empty() {
                        None
                    } else {
                        Some(costs.iter().sum::<f64>() / costs.len() as f64)
                    }
                };
                r.cost_estimated = list.is_empty() || list.iter().any(|m| table.is_estimated(m));
                if let Some(c) = cost {
                    total_cost += c;
                    any = true;
                }
                r.cost_usd = cost;
                r
            })
            .collect();
        totals.cost_usd = any.then_some(total_cost);
        (rows, totals)
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

/// Model used to price a ticket whose agents may differ; the first agent's
/// model is a good enough proxy for the board column.
fn dominant_model(agents: &[orchestra_core::model::Agent]) -> String {
    agents
        .iter()
        .find(|a| !a.model.is_empty())
        .map(|a| a.model.clone())
        .unwrap_or_default()
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
