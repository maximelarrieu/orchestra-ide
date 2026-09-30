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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use orchestra_core::claude::StreamLine;
use orchestra_core::config::Config;
use orchestra_core::events::{EventKind, NewEvent};
use orchestra_core::guard::GitPolicy;
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

/// The shape every pull request description follows. Handed to the integrator
/// when, and only when, a request is what this run will produce.

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
    /// Global conventions. Read afresh for every agent, so a rule accepted
    /// while a ticket runs reaches the next agent of that ticket.
    conventions_dir: PathBuf,
    running: Arc<Mutex<HashMap<AgentId, Running>>>,
    tickets: Arc<Mutex<HashMap<TicketId, tokio::task::JoinHandle<()>>>>,
    /// Tickets launched with `open_panes`, which get a pane per agent even
    /// when `zellij.auto_pane` is off. Per run, not per configuration: the
    /// choice belongs to the launch that asked for it.
    panes_wanted: Arc<Mutex<HashSet<TicketId>>>,
}

impl Supervisor {
    pub fn new(
        store: Store,
        bus: EventBus,
        ledger: UsageLedger,
        cfg: Arc<Config>,
        cache_dir: PathBuf,
        conventions_dir: PathBuf,
    ) -> Self {
        Supervisor {
            store,
            bus,
            ledger,
            cfg,
            cache_dir,
            conventions_dir,
            running: Arc::new(Mutex::new(HashMap::new())),
            tickets: Arc::new(Mutex::new(HashMap::new())),
            panes_wanted: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn is_running(&self, ticket_id: TicketId) -> bool {
        self.tickets.lock().await.contains_key(&ticket_id)
    }

    pub async fn running_agents(&self) -> usize {
        self.running.lock().await.len()
    }

    /// Start the team of a planned ticket.
    pub async fn launch(
        &self,
        ticket: Ticket,
        project: Project,
        catalog: Catalog,
        open_panes: bool,
    ) -> Result<()> {
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

        let mut ticket = ticket;
        let (branch, worktree_path) = self.ensure_worktree(&mut ticket, &project).await?;

        let from = ticket.status;
        check_transition(from, TicketStatus::Running)?;
        ticket.branch = Some(branch);
        ticket.worktree_path = Some(worktree_path.clone());
        ticket.status = TicketStatus::Running;
        ticket.updated_at = orchestra_core::now();
        self.store.update_ticket(ticket.clone()).await?;

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
        if open_panes {
            self.panes_wanted.lock().await.insert(ticket_id);
        }
        let handle = tokio::spawn(async move {
            let outcome = me
                .run_team(ticket, project, team, catalog, worktree_path)
                .await;
            if let Err(e) = outcome {
                tracing::error!("exécution du ticket interrompue : {e:#}");
            }
            me.tickets.lock().await.remove(&ticket_id);
            me.panes_wanted.lock().await.remove(&ticket_id);
        });
        self.tickets.lock().await.insert(ticket_id, handle);
        Ok(())
    }

    /// The ticket's worktree, recreated from its branch if it is not there.
    ///
    /// A worktree is a checkout, not the work: it is deleted once a ticket is
    /// integrated, and the branch is what holds the commits. So a ticket picked
    /// up again — reopened, integrated a second way — gets its checkout back
    /// instead of a dead end.
    pub async fn ensure_worktree(
        &self,
        ticket: &mut Ticket,
        project: &Project,
    ) -> Result<(String, PathBuf)> {
        if let (Some(branch), Some(path)) = (ticket.branch.clone(), ticket.worktree_path.clone()) {
            if path.exists() {
                return Ok((branch, path));
            }
        }
        anyhow::ensure!(
            worktree::is_repository(&project.path),
            "{} n'est pas un dépôt git",
            project.path.display()
        );
        // A project created an hour ago has nothing in it. Saying so matters:
        // this is the only commit Orchestra ever writes outside a worktree.
        if worktree::ensure_root_commit(project)? {
            self.warn_ticket(
                ticket.id,
                project.id,
                format!(
                    "{} n'avait aucun commit : un commit vide « init » a été créé pour que \
                     « {} » existe",
                    project.name, project.default_branch
                ),
            )
            .await;
        }
        let plan = worktree::plan_for(
            project,
            ticket,
            &self
                .cfg
                .daemon
                .worktrees_dir
                .clone()
                .unwrap_or_else(orchestra_core::config::Paths::worktrees_dir),
            &self.cfg.daemon.branch_prefix,
        );
        let _ = worktree::prune(project);
        let wt = worktree::ensure(project, &plan)?;
        ticket.branch = Some(wt.branch.clone());
        ticket.worktree_path = Some(wt.path.clone());
        ticket.updated_at = orchestra_core::now();
        self.store.update_ticket(ticket.clone()).await?;
        let _ = self
            .bus
            .publish(
                NewEvent::new(EventKind::WorktreeCreated {
                    path: wt.path.clone(),
                    branch: wt.branch.clone(),
                })
                .project(project.id)
                .ticket(ticket.id),
            )
            .await;
        Ok((wt.branch, wt.path))
    }

    /// Launch the integrator on a ticket the relecture cleared.
    ///
    /// It is the only agent that runs after a ticket has left `Running`, and
    /// the only one whose guard opens git. It still works in the worktree: the
    /// fusion into the default branch is done here, in Rust, once the agent has
    /// made the branch ready.
    pub async fn integrate(
        &self,
        ticket: Ticket,
        project: Project,
        catalog: Catalog,
    ) -> Result<()> {
        let cfg = self.cfg.integration.clone();
        anyhow::ensure!(
            cfg.enabled,
            "l'intégration est désactivée dans la configuration"
        );
        anyhow::ensure!(
            catalog.get(&cfg.role).is_some(),
            "le rôle « {} » est absent du catalogue : lance « orchestra init »",
            cfg.role
        );
        anyhow::ensure!(!self.is_running(ticket.id).await, "ce ticket tourne déjà");
        // The gate is not decoration on a screen: a branch whose checks are
        // red does not reach the default branch, whichever client asks. The
        // TUI hides the key; this is what makes hiding it beside the point.
        if let Some(last) = self.last_checks(ticket.id).await {
            if let Some(failed) = last.failed() {
                anyhow::bail!(
                    "les vérifications du dépôt refusent cette branche : {}",
                    failed.label_fr()
                );
            }
        }
        let mut ticket = ticket;
        let (branch, worktree_path) = self.ensure_worktree(&mut ticket, &project).await?;

        // A branch the integrator already prepared, refused only at the door:
        // if nothing has moved since, the door is all there is to retry.
        // Spending an agent to find the branch exactly as it left it would buy
        // nothing. The default branch moving is the one case that needs it
        // again — someone has to bring it in.
        let ready = match crate::integration::blocked_merge(&self.store, ticket.id).await {
            Some(blocked) => {
                blocked.branch == branch
                    && worktree::branch_head(&project.path, &branch).as_deref()
                        == Some(blocked.head.as_str())
                    && worktree::fast_forwardable(&project, &branch)
            }
            None => false,
        };

        let me = self.clone();
        let ticket_id = ticket.id;
        let handle = tokio::spawn(async move {
            let outcome = if ready {
                me.deliver(ticket, project, worktree_path, branch).await
            } else {
                me.run_integration(ticket, project, catalog, worktree_path, branch)
                    .await
            };
            if let Err(e) = outcome {
                tracing::error!("intégration interrompue : {e:#}");
            }
            me.tickets.lock().await.remove(&ticket_id);
        });
        self.tickets.lock().await.insert(ticket_id, handle);
        Ok(())
    }

    async fn run_integration(
        &self,
        ticket: Ticket,
        project: Project,
        catalog: Catalog,
        worktree_path: PathBuf,
        branch: String,
    ) -> Result<()> {
        let cfg = self.cfg.integration.clone();
        // What the team said, read back from the store: the task that held it
        // in memory ended when the ticket went to relecture.
        let past = self.store.agents_of_ticket(ticket.id).await?;
        let stage = past.iter().map(|a| a.stage).max().unwrap_or(0) + 1;
        let mut handoffs: Vec<(String, String)> = past
            .into_iter()
            .filter_map(|a| a.handoff.map(|h| (a.role, h)))
            .collect();

        let member = orchestra_core::model::TeamMember {
            role: cfg.role.clone(),
            objective: integration_objective(&project.default_branch, &branch, cfg.push, cfg.mode),
            depends_on: Vec::new(),
            model: None,
            effort: None,
            max_budget_usd: None,
            parallel_ok: false,
        };

        // The pull request template is a convention with `mode: pr`: it only
        // reaches the integrator, and only where a request is what comes out.
        let mut stage = stage;
        let step = self
            .run_member(Step {
                ticket: &ticket,
                project: &project,
                catalog: &catalog,
                member: member.clone(),
                stage,
                worktree_path: &worktree_path,
                handoffs: &mut handoffs,
                blocking: &[],
                blocked_by: Blocked::Review,
                git: self.git_for(&catalog, &member.role),
                appendix: "",
            })
            .await;
        if !matches!(step, StepOutcome::Done) {
            self.warn_ticket(
                ticket.id,
                project.id,
                "l'intégration s'est arrêtée avant d'avoir préparé la branche : \
                 le ticket reste à relire"
                    .into(),
            )
            .await;
            return Ok(());
        }
        stage += 1;

        // What the conventions measure is measured, not asked: the branch
        // leaves this machine only once its commits and its description hold.
        if !self
            .rules_gate(
                &ticket,
                &project,
                &catalog,
                &member,
                &branch,
                &worktree_path,
                &mut handoffs,
                &mut stage,
            )
            .await
        {
            return Ok(());
        }

        self.deliver(ticket, project, worktree_path, branch).await
    }

    /// Send a prepared branch out: a pull request, or the fusion and its push.
    ///
    /// Everything after the integrator, so that a branch refused only at the
    /// fusion can come back here without one.
    async fn deliver(
        &self,
        ticket: Ticket,
        project: Project,
        worktree_path: PathBuf,
        branch: String,
    ) -> Result<()> {
        let cfg = self.cfg.integration.clone();
        let handoffs: Vec<(String, String)> = self
            .store
            .agents_of_ticket(ticket.id)
            .await?
            .into_iter()
            .filter_map(|a| a.handoff.map(|h| (a.role, h)))
            .collect();

        // A pull request replaces the fusion: the branch goes up, GitHub holds
        // it, and the merge is the user's click. Falling back to the local
        // fusion when there is nowhere to open one is better than stopping.
        if cfg.mode.is_pr() {
            match self
                .open_pull_request(&ticket, &project, &branch, &worktree_path, &handoffs)
                .await
            {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(e) => {
                    self.warn_ticket(
                        ticket.id,
                        project.id,
                        format!("pull request impossible : {e:#} — fusion locale à la place"),
                    )
                    .await;
                }
            }
        }

        // The fusion itself is ours. An agent that fails leaves the default
        // branch exactly where it was.
        let commits = match worktree::merge_fast_forward(&project, &branch) {
            Ok(commits) => commits,
            Err(e) => {
                // Not a warning: the branch is ready and the ticket now waits
                // on the user, which the board has to be able to say.
                let reason = format!("{e:#}");
                tracing::warn!("fusion de {branch} refusée : {reason}");
                let head = worktree::branch_head(&project.path, &branch).unwrap_or_default();
                let _ = self
                    .bus
                    .publish(
                        NewEvent::new(EventKind::MergeBlocked {
                            branch: branch.clone(),
                            head,
                            reason,
                        })
                        .project(project.id)
                        .ticket(ticket.id),
                    )
                    .await;
                return Ok(());
            }
        };

        // The integrator pushes its own branch when asked, but nothing was
        // pushing the branch that just received it: the fusion is ours, so is
        // sending it out. Off unless `integration.push` says otherwise —
        // publishing is outward-facing and stays the user's call.
        let pushed_to = if cfg.push {
            match worktree::push_branch(&project.path, &project.default_branch) {
                Ok(remote) => remote,
                Err(e) => {
                    self.warn_ticket(
                        ticket.id,
                        project.id,
                        format!(
                            "fusion faite, mais le push a échoué : {e:#}. La branche par \
                             défaut est à jour en local."
                        ),
                    )
                    .await;
                    None
                }
            }
        } else {
            None
        };

        let _ = self
            .bus
            .publish(
                NewEvent::new(EventKind::TicketMerged {
                    branch: branch.clone(),
                    into: project.default_branch.clone(),
                    commits,
                    pushed_to,
                })
                .project(project.id)
                .ticket(ticket.id),
            )
            .await;

        if cfg.remove_worktree {
            match worktree::remove(&project, &worktree_path) {
                Ok(()) => {
                    // Nothing must keep pointing at a directory that is gone.
                    if let Ok(Some(mut current)) = self.store.ticket(ticket.id).await {
                        current.worktree_path = None;
                        current.updated_at = orchestra_core::now();
                        let _ = self.store.update_ticket(current).await;
                    }
                }
                Err(e) => {
                    self.warn_ticket(
                        ticket.id,
                        project.id,
                        format!("worktree non supprimé : {e:#}"),
                    )
                    .await;
                }
            }
        }

        self.finish_ticket(&ticket, TicketStatus::Done).await;
        Ok(())
    }

    /// Push the branch and open a pull request for it.
    ///
    /// Returns false when this project cannot have one — no remote, or no `gh`
    /// — so the caller falls back to the local fusion.
    async fn open_pull_request(
        &self,
        ticket: &Ticket,
        project: &Project,
        branch: &str,
        worktree_path: &Path,
        handoffs: &[(String, String)],
    ) -> Result<bool> {
        if worktree::default_remote(&project.path).is_none() {
            self.warn_ticket(
                ticket.id,
                project.id,
                "ce dépôt n'a pas de remote : pas de pull request possible".into(),
            )
            .await;
            return Ok(false);
        }
        if !crate::github::is_available() {
            self.warn_ticket(
                ticket.id,
                project.id,
                "« gh » est introuvable : installe-le pour ouvrir des pull requests".into(),
            )
            .await;
            return Ok(false);
        }

        worktree::push_branch(&project.path, branch)?;
        let title = format!("#{} {}", ticket.number, ticket.title);
        // What the integrator wrote following the template, if it did. The
        // assembled version is the floor, not the goal: an agent that has just
        // read the whole branch writes a better description than a format
        // string can.
        let body = match written_pr_body(worktree_path) {
            Some(text) => text,
            None => assembled_pr_body(ticket, handoffs),
        };
        let body = format!("{body}\n\n{}", self.pr_footer(ticket, branch).await);
        let pr = crate::github::create_pr(
            &project.path,
            &project.default_branch,
            branch,
            &title,
            &body,
        )?;
        let _ = self
            .bus
            .publish(
                NewEvent::new(EventKind::PullRequestOpened {
                    url: pr.url.clone(),
                    number: pr.number,
                })
                .project(project.id)
                .ticket(ticket.id),
            )
            .await;
        // The ticket stays « à relire » on purpose: the work is not in the
        // default branch until someone merges. The watcher below closes it.
        Ok(true)
    }

    /// Follow the pull requests we opened until GitHub answers.
    ///
    /// Polling rather than a webhook: the daemon is a local process on a laptop
    /// with no address to be called back at, and one `gh` call a minute per
    /// open request is nothing.
    pub async fn watch_pull_requests(self, cancel: tokio_util::sync::CancellationToken) {
        let every = std::time::Duration::from_secs(self.cfg.integration.pr_poll_secs.max(10));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(every) => {}
            }
            if let Err(e) = self.poll_pull_requests().await {
                tracing::debug!("suivi des pull requests : {e:#}");
            }
        }
    }

    async fn poll_pull_requests(&self) -> Result<()> {
        let waiting = self
            .store
            .list_tickets(None, Some(vec![TicketStatus::Review]))
            .await?;
        for ticket in waiting {
            let Some(url) = crate::github::open_pull_request(&self.store, ticket.id).await else {
                continue;
            };
            let Some(project) = self.store.project(ticket.project_id).await? else {
                continue;
            };
            // A failure here is usually being offline; the next round asks
            // again rather than deciding anything.
            let state = match crate::github::pr_state(&project.path, &url) {
                Ok(state) => state,
                Err(e) => {
                    tracing::debug!("état de {url} indisponible : {e:#}");
                    continue;
                }
            };
            match state {
                crate::github::PrState::Open => continue,
                crate::github::PrState::Merged => {
                    self.close_pull_request(&ticket, &project, &url, true).await
                }
                crate::github::PrState::Closed => {
                    self.close_pull_request(&ticket, &project, &url, false)
                        .await
                }
            }
        }
        Ok(())
    }

    async fn close_pull_request(
        &self,
        ticket: &Ticket,
        project: &Project,
        url: &str,
        merged: bool,
    ) {
        let _ = self
            .bus
            .publish(
                NewEvent::new(EventKind::PullRequestClosed {
                    url: url.to_string(),
                    merged,
                })
                .project(project.id)
                .ticket(ticket.id),
            )
            .await;

        if !merged {
            self.warn_ticket(
                ticket.id,
                project.id,
                format!(
                    "la pull request du ticket #{} a été fermée sans fusion : la branche \
                     reste, le ticket est annulé",
                    ticket.number
                ),
            )
            .await;
            self.finish_ticket(ticket, TicketStatus::Cancelled).await;
            return;
        }

        // The merge happened over there; this machine has to catch up before
        // the default branch can move again.
        if let Err(e) = worktree::pull_default_branch(project) {
            self.warn_ticket(
                ticket.id,
                project.id,
                format!(
                    "pull request fusionnée, mais « {} » n'a pas pu être mise à jour ici : {e:#}",
                    project.default_branch
                ),
            )
            .await;
        }
        if self.cfg.integration.remove_worktree {
            if let Some(path) = ticket.worktree_path.clone() {
                if worktree::remove(project, &path).is_ok() {
                    if let Ok(Some(mut current)) = self.store.ticket(ticket.id).await {
                        current.worktree_path = None;
                        current.updated_at = orchestra_core::now();
                        let _ = self.store.update_ticket(current).await;
                    }
                }
            }
        }
        self.finish_ticket(ticket, TicketStatus::Done).await;
    }

    /// Walk the stages, one role at a time, then hold the relecture loop.
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
        let mut next_stage = 0u32;
        // The gate runs immediately before every relecture, so the last thing
        // to happen before a verdict is always a measurement. This says
        // whether the stage loop already did it.
        let mut gated = false;

        // A ticket relaunched after an interruption picks up where it stopped.
        // A rôle that already finished has its work in the branch and its
        // handoff in the store; running it again would pay a second time for
        // what is already there, and let it undo its own work.
        let prior = self
            .store
            .agents_of_ticket(ticket.id)
            .await
            .unwrap_or_default();
        let done_before: Vec<String> = team
            .ordered()
            .iter()
            .filter(|(_, m)| finished_handoff(&prior, &m.role).is_some())
            .map(|(_, m)| m.role.clone())
            .collect();
        if !done_before.is_empty() {
            let _ = self
                .bus
                .publish(
                    NewEvent::new(EventKind::TicketResumed {
                        skipped: done_before.clone(),
                    })
                    .project(project.id)
                    .ticket(ticket.id),
                )
                .await;
        }

        for (stage, member) in team.ordered() {
            next_stage = stage + 1;
            if let Some(handoff) = finished_handoff(&prior, &member.role) {
                handoffs.push((member.role.clone(), handoff));
                continue;
            }
            // The relecture is a team member like the others, so this is where
            // the gate belongs: right before it, never after. Reading a branch
            // that does not build is an hour of an expensive agent spent on
            // what `cargo build` says in twenty seconds.
            let mut appendix = String::new();
            if member.role == self.cfg.review.role {
                let mut gate_stage = stage;
                match self
                    .checks_gate(
                        &ticket,
                        &project,
                        &team,
                        &catalog,
                        &worktree_path,
                        &mut handoffs,
                        &mut gate_stage,
                    )
                    .await
                {
                    StepOutcome::Done => {}
                    StepOutcome::Cancelled => {
                        self.finish_ticket(&ticket, TicketStatus::Cancelled).await;
                        return Ok(());
                    }
                    StepOutcome::Failed => {
                        failed = true;
                        break;
                    }
                }
                gated = true;
                next_stage = gate_stage.max(stage) + 1;
                if let Some(outcome) = self.last_checks(ticket.id).await {
                    appendix = checks_appendix(&outcome);
                }
            }
            let step = self
                .run_member(Step {
                    ticket: &ticket,
                    project: &project,
                    catalog: &catalog,
                    member: member.clone(),
                    stage,
                    worktree_path: &worktree_path,
                    handoffs: &mut handoffs,
                    blocking: &[],
                    blocked_by: Blocked::Review,
                    git: self.git_for(&catalog, &member.role),
                    appendix: &appendix,
                })
                .await;
            match step {
                StepOutcome::Done => {}
                StepOutcome::Cancelled => {
                    self.finish_ticket(&ticket, TicketStatus::Cancelled).await;
                    return Ok(());
                }
                StepOutcome::Failed => {
                    failed = true;
                    break;
                }
            }
        }

        // A team whose reviewer was taken out still gets its gate: the branch
        // is what leaves, whoever read it.
        if !failed && !gated {
            match self
                .checks_gate(
                    &ticket,
                    &project,
                    &team,
                    &catalog,
                    &worktree_path,
                    &mut handoffs,
                    &mut next_stage,
                )
                .await
            {
                StepOutcome::Done => {}
                StepOutcome::Cancelled => {
                    self.finish_ticket(&ticket, TicketStatus::Cancelled).await;
                    return Ok(());
                }
                StepOutcome::Failed => failed = true,
            }
        }

        if !failed {
            // A verdict already on record must not be published again: it would
            // count as a new round and read as if the relecture had just spoken.
            let verdict_is_fresh = !done_before.contains(&self.cfg.review.role);
            match self
                .review_rounds(
                    &ticket,
                    &project,
                    &team,
                    &catalog,
                    &worktree_path,
                    &mut handoffs,
                    next_stage,
                    verdict_is_fresh,
                )
                .await
            {
                StepOutcome::Done => {}
                StepOutcome::Cancelled => {
                    self.finish_ticket(&ticket, TicketStatus::Cancelled).await;
                    return Ok(());
                }
                StepOutcome::Failed => failed = true,
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

    /// Run one member, and fold its handoff into the ticket's memory.
    async fn run_member(&self, step: Step<'_>) -> StepOutcome {
        let Step {
            ticket,
            project,
            catalog,
            member,
            stage,
            worktree_path,
            handoffs,
            blocking,
            blocked_by,
            git,
            appendix,
        } = step;
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
            return StepOutcome::Failed;
        };

        let outcome = self
            .run_role(RoleRun {
                ticket,
                project,
                role,
                member: &member,
                stage,
                worktree_path,
                handoffs,
                blocking,
                blocked_by,
                git,
                appendix,
            })
            .await;

        match outcome {
            Ok(AgentOutcome::Done { handoff }) => {
                self.record_proposals(ticket, project, &member.role, &handoff)
                    .await;
                handoffs.push((member.role.clone(), handoff));
                StepOutcome::Done
            }
            Ok(AgentOutcome::Cancelled) => StepOutcome::Cancelled,
            Ok(AgentOutcome::Failed { reason }) => {
                self.bus
                    .warn(format!(
                        "l'agent « {} » s'est arrêté : {reason}",
                        member.role
                    ))
                    .await;
                StepOutcome::Failed
            }
            Err(e) => {
                self.bus
                    .warn(format!(
                        "l'agent « {} » n'a pas pu tourner : {e:#}",
                        member.role
                    ))
                    .await;
                StepOutcome::Failed
            }
        }
    }

    /// Write down the rules an agent proposed at the end of its message.
    ///
    /// Written by us, with `status: proposed`, in the project's `.orchestra`
    /// directory: the agent never touches it, and nothing applies a proposal
    /// before the user has accepted it. A proposal that cannot be written costs
    /// a warning, never the ticket.
    async fn record_proposals(&self, ticket: &Ticket, project: &Project, role: &str, handoff: &str) {
        for proposal in orchestra_core::conventions::parse_proposals(handoff) {
            let by = format!("{role}, ticket #{}", ticket.number);
            let draft = orchestra_core::conventions::RuleDraft {
                kind: proposal.kind,
                title: &proposal.title,
                status: orchestra_core::conventions::RuleStatus::Proposed,
                applies_to: &proposal.applies_to,
                proposed_by: Some(&by),
                body: &proposal.body,
            };
            match crate::rules::create(&self.conventions_dir, Some(project), &draft) {
                Ok((name, _)) => {
                    self.publish_ticket(
                        ticket,
                        EventKind::RuleProposed {
                            rule_kind: proposal.kind,
                            name,
                            title: proposal.title.clone(),
                            by,
                        },
                    )
                    .await;
                }
                Err(e) => {
                    self.warn_ticket(
                        ticket.id,
                        project.id,
                        format!("proposition « {} » non enregistrée : {e:#}", proposal.title),
                    )
                    .await;
                }
            }
        }
    }

    /// The measured conventions, evaluated on the branch the integrator just
    /// prepared. True when it may go on to the fusion or the pull request.
    ///
    /// A violation sends the integrator back — it is the only agent git is
    /// open to, so the only one that can reword a commit — for at most
    /// `checks.max_rounds` rounds. Past that the branch stays where it is:
    /// an agent saying it is fine does not outweigh a message that does not
    /// match.
    #[allow(clippy::too_many_arguments)]
    async fn rules_gate(
        &self,
        ticket: &Ticket,
        project: &Project,
        catalog: &Catalog,
        member: &orchestra_core::model::TeamMember,
        branch: &str,
        worktree_path: &Path,
        handoffs: &mut Vec<(String, String)>,
        stage: &mut u32,
    ) -> bool {
        let is_pr = self.cfg.integration.mode.is_pr();
        let mut round = 1;
        loop {
            let book = crate::rules::book(&self.conventions_dir, Some(project));
            if round == 1 {
                for (path, err) in &book.errors {
                    self.warn_ticket(
                        ticket.id,
                        project.id,
                        format!("règle illisible, donc non vérifiée : {} — {err}", path.display()),
                    )
                    .await;
                }
            }
            if book.checks(is_pr).is_empty() {
                return true;
            }
            let facts = orchestra_core::conventions::BranchFacts {
                commits: match worktree::commit_messages(project, branch) {
                    Ok(commits) => commits,
                    Err(e) => {
                        self.warn_ticket(
                            ticket.id,
                            project.id,
                            format!("historique illisible, conventions non vérifiées : {e:#}"),
                        )
                        .await;
                        return false;
                    }
                },
                pr_body: written_pr_body(worktree_path),
            };
            let violations = orchestra_core::conventions::evaluate_all(&book, is_pr, &facts);
            self.publish_ticket(
                ticket,
                EventKind::RulesChecked {
                    round,
                    violations: violations.clone(),
                },
            )
            .await;
            if violations.is_empty() {
                return true;
            }
            if round > self.cfg.checks.max_rounds {
                self.warn_ticket(
                    ticket.id,
                    project.id,
                    format!(
                        "la branche ne respecte toujours pas les conventions après {} tour(s) \
                         ({}) : ni fusion ni pull request, le ticket reste à relire",
                        round - 1,
                        violations[0].line()
                    ),
                )
                .await;
                return false;
            }
            let items: Vec<String> = violations.iter().map(|v| v.line()).collect();
            let mut again = member.clone();
            again.objective = correction_objective(&member.objective, &items, Blocked::Rules);
            let step = self
                .run_member(Step {
                    ticket,
                    project,
                    catalog,
                    member: again,
                    stage: *stage,
                    worktree_path,
                    handoffs,
                    blocking: &items,
                    blocked_by: Blocked::Rules,
                    git: self.git_for(catalog, &member.role),
                    appendix: "",
                })
                .await;
            if !matches!(step, StepOutcome::Done) {
                self.warn_ticket(
                    ticket.id,
                    project.id,
                    "l'intégrateur s'est arrêté pendant la mise en conformité : le ticket \
                     reste à relire"
                        .into(),
                )
                .await;
                return false;
            }
            *stage += 1;
            round += 1;
        }
    }

    /// The repository's own checks, run in the ticket's worktree.
    ///
    /// This is the one claim of the whole run that nobody had to be asked for.
    /// The relecture *reports* that the tests pass; this *measures* it, and an
    /// exit code cannot be optimistic. So it runs before the reviewer is paid
    /// for reading a branch that does not build, and again after any
    /// correction round, so that « à relire » means what it says.
    ///
    /// A red gate sends back the last role that wrote code, with the command's
    /// own output rather than a summary of it. Still red once the budget is
    /// spent, the ticket fails — which is what happened: the team did not
    /// deliver something that builds. It can be relaunched, and the relaunch
    /// keeps the work that is already in the branch.
    #[allow(clippy::too_many_arguments)]
    async fn checks_gate(
        &self,
        ticket: &Ticket,
        project: &Project,
        team: &Team,
        catalog: &Catalog,
        worktree_path: &Path,
        handoffs: &mut Vec<(String, String)>,
        stage: &mut u32,
    ) -> StepOutcome {
        let cfg = &self.cfg.checks;
        let commands = crate::checks::commands(cfg, &project.path);
        if commands.is_empty() {
            return StepOutcome::Done;
        }
        let timeout = std::time::Duration::from_secs(cfg.timeout_secs.max(1));
        // The reviewer is not sent back to fix a build: it did not write it.
        let workers: Vec<String> = team
            .members
            .iter()
            .map(|m| m.role.clone())
            .filter(|r| r != &self.cfg.review.role)
            .collect();

        // A resumed ticket does not get its repair budget back, for the same
        // reason a resumed relecture does not: the rounds were paid for.
        let (last_round, mut spent) = self.checks_record(ticket.id).await;
        let mut round = last_round + 1;

        loop {
            let mut runs = Vec::new();
            for check in &commands {
                self.publish_ticket(
                    ticket,
                    EventKind::CheckStarted {
                        round,
                        command: check.label(),
                    },
                )
                .await;
                let run = crate::checks::run(check, worktree_path, timeout).await;
                let refused = !run.ok;
                self.publish_ticket(
                    ticket,
                    EventKind::CheckFinished {
                        round,
                        run: Box::new(run.clone()),
                    },
                )
                .await;
                runs.push(run);
                // The first refusal ends the pass: what follows would only
                // report on a tree already known not to hold.
                if refused {
                    break;
                }
            }

            let outcome = orchestra_core::checks::ChecksOutcome { round, runs };
            if outcome.passed() {
                return StepOutcome::Done;
            }
            let Some(failure) = outcome.failed().cloned() else {
                return StepOutcome::Done;
            };

            let Some(role) = last_worker(handoffs, &workers) else {
                self.warn_ticket(
                    ticket.id,
                    project.id,
                    format!(
                        "{} et personne dans l'équipe ne peut le reprendre : le ticket \
                         passe sous tes yeux",
                        failure.label_fr()
                    ),
                )
                .await;
                return StepOutcome::Failed;
            };
            if spent >= cfg.max_rounds {
                self.warn_ticket(
                    ticket.id,
                    project.id,
                    format!(
                        "{} encore après {spent} tour(s) de réparation : le ticket \
                         s'arrête là plutôt que d'en payer un de plus",
                        failure.label_fr()
                    ),
                )
                .await;
                return StepOutcome::Failed;
            }
            spent += 1;

            let Some(base) = team.member(&role).cloned() else {
                return StepOutcome::Failed;
            };
            let items = outcome.blocking_lines();
            let mut member = base.clone();
            member.objective = correction_objective(&base.objective, &items, Blocked::Checks);
            let step = self
                .run_member(Step {
                    ticket,
                    project,
                    catalog,
                    member,
                    stage: *stage,
                    worktree_path,
                    handoffs,
                    blocking: &items,
                    blocked_by: Blocked::Checks,
                    git: self.git_for(catalog, &role),
                    appendix: "",
                })
                .await;
            if !matches!(step, StepOutcome::Done) {
                return step;
            }
            *stage += 1;
            round += 1;
        }
    }

    /// What the gate said on its last pass, `None` when it never ran.
    pub async fn last_checks(
        &self,
        ticket_id: TicketId,
    ) -> Option<orchestra_core::checks::ChecksOutcome> {
        let filter = orchestra_core::events::EventFilter {
            ticket_id: Some(ticket_id),
            tags: vec![orchestra_core::events::EventTag::CheckFinished],
            ..Default::default()
        };
        let events = self
            .store
            .recent_events(filter, 64)
            .await
            .unwrap_or_default();
        crate::checks::last_pass(&events)
    }

    /// The highest pass this ticket has run, and how many of them came back
    /// red. The second is the repair budget already spent.
    async fn checks_record(&self, ticket_id: TicketId) -> (u32, u32) {
        let filter = orchestra_core::events::EventFilter {
            ticket_id: Some(ticket_id),
            tags: vec![orchestra_core::events::EventTag::CheckFinished],
            ..Default::default()
        };
        let events = self
            .store
            .recent_events(filter, 64)
            .await
            .unwrap_or_default();
        let mut last = 0u32;
        let mut red: Vec<u32> = Vec::new();
        for event in events {
            let EventKind::CheckFinished { round, run } = event.kind else {
                continue;
            };
            last = last.max(round);
            if !run.ok && !red.contains(&round) {
                red.push(round);
            }
        }
        (last, red.len() as u32)
    }

    /// One event about the ticket rather than about an agent.
    async fn publish_ticket(&self, ticket: &Ticket, kind: EventKind) {
        let _ = self
            .bus
            .publish(
                NewEvent::new(kind)
                    .project(ticket.project_id)
                    .ticket(ticket.id),
            )
            .await;
    }

    /// Read the relecture's verdict and, while it blocks, send the named roles
    /// back to work and have it read the branch again.
    ///
    /// The loop is bounded twice over: by `review.max_rounds`, and by the
    /// verdict itself — a relecture that says nothing readable ends it, because
    /// silence must never be read as an approval.
    #[allow(clippy::too_many_arguments)]
    async fn review_rounds(
        &self,
        ticket: &Ticket,
        project: &Project,
        team: &Team,
        catalog: &Catalog,
        worktree_path: &Path,
        handoffs: &mut Vec<(String, String)>,
        first_stage: u32,
        verdict_is_fresh: bool,
    ) -> StepOutcome {
        let cfg = &self.cfg.review;
        let Some(reviewer) = team.member(&cfg.role).cloned() else {
            return StepOutcome::Done;
        };
        if !cfg.enabled {
            return StepOutcome::Done;
        }
        let workers: Vec<String> = team
            .members
            .iter()
            .map(|m| m.role.clone())
            .filter(|r| r != &cfg.role)
            .collect();
        if workers.is_empty() {
            return StepOutcome::Done;
        }

        let mut stage = first_stage;
        // Each blocking verdict opened exactly one correction round, so the
        // record says how many have been spent — a resumed ticket does not get
        // its budget back.
        let mut rounds_done = self.blocking_verdicts(ticket.id).await;
        let mut fresh = verdict_is_fresh;
        loop {
            let Some(text) = handoffs
                .iter()
                .rev()
                .find(|(r, _)| r == &cfg.role)
                .map(|(_, t)| t.clone())
            else {
                return StepOutcome::Done;
            };
            let Some(review) = orchestra_core::review::parse_review(&text) else {
                self.warn_ticket(
                    ticket.id,
                    project.id,
                    "la relecture n'a pas terminé par un verdict lisible : le ticket s'arrête \
                     ici plutôt que de relancer l'équipe sur une lecture incertaine"
                        .into(),
                )
                .await;
                return StepOutcome::Done;
            };

            // Who goes back to work. A blocking point that names nobody is for
            // the last role that wrote code: it holds the freshest context, and
            // waking the whole team on an unattributed remark costs more than
            // it repairs.
            let named = review.roles_to_fix(&workers);
            let mut to_fix: Vec<String> = if review.verdict.is_ready() {
                Vec::new()
            } else if named.is_empty() {
                workers.last().cloned().into_iter().collect()
            } else {
                named
            };

            if fresh {
                let _ = self
                    .bus
                    .publish(
                        NewEvent::new(EventKind::ReviewVerdict {
                            round: rounds_done + 1,
                            verdict: review.verdict,
                            blocking: review.blocking_lines(),
                            roles: to_fix.clone(),
                        })
                        .project(project.id)
                        .ticket(ticket.id),
                    )
                    .await;
            }

            if review.verdict.is_ready() {
                return StepOutcome::Done;
            }

            // On a resumed ticket, the roles this verdict named may already
            // have come back to work before the interruption. Their commits
            // are in the branch; only the relecture that would have checked
            // them is missing.
            let settled = corrected_since_review(
                &self
                    .store
                    .agents_of_ticket(ticket.id)
                    .await
                    .unwrap_or_default(),
                &cfg.role,
            );
            let redone: Vec<String> = to_fix
                .iter()
                .filter(|r| settled.contains(r))
                .cloned()
                .collect();
            to_fix.retain(|r| !settled.contains(r));

            if to_fix.is_empty() {
                if !redone.is_empty() {
                    self.warn_ticket(
                        ticket.id,
                        project.id,
                        format!(
                            "« {} » avait déjà corrigé avant l'interruption : on enchaîne \
                             sur la relecture",
                            redone.join(", ")
                        ),
                    )
                    .await;
                }
            } else {
                if rounds_done >= cfg.max_rounds {
                    self.warn_ticket(
                        ticket.id,
                        project.id,
                        format!(
                            "la relecture bloque encore après {rounds_done} tour(s) de \
                             correction : le ticket passe en relecture humaine"
                        ),
                    )
                    .await;
                    return StepOutcome::Done;
                }
                rounds_done += 1;
            }

            for role in &to_fix {
                let Some(base) = team.member(role) else {
                    continue;
                };
                let items = blocking_for(&review, role, &to_fix);
                let mut member = base.clone();
                member.objective = correction_objective(&base.objective, &items, Blocked::Review);
                let step = self
                    .run_member(Step {
                        ticket,
                        project,
                        catalog,
                        member,
                        stage,
                        worktree_path,
                        handoffs,
                        blocking: &items,
                        blocked_by: Blocked::Review,
                        git: self.git_for(catalog, role),
                        appendix: "",
                    })
                    .await;
                if !matches!(step, StepOutcome::Done) {
                    return step;
                }
                stage += 1;
            }

            // The gate again, and for the same reason as the first time: the
            // roles that just came back rewrote code, and the relecture about
            // to read it should not be the one to find out that it no longer
            // builds. Running it here rather than at the end is what makes the
            // rule hold — before every verdict, a measurement.
            match self
                .checks_gate(
                    ticket,
                    project,
                    team,
                    catalog,
                    worktree_path,
                    handoffs,
                    &mut stage,
                )
                .await
            {
                StepOutcome::Done => {}
                other => return other,
            }
            let appendix = match self.last_checks(ticket.id).await {
                Some(outcome) => checks_appendix(&outcome),
                None => String::new(),
            };

            let mut again = reviewer.clone();
            again.objective = format!(
                "Relire à nouveau la branche après le tour de correction {rounds_done} : \
                 vérifier que chaque point que tu avais bloqué est levé, puis rendre ton \
                 verdict.",
            );
            let step = self
                .run_member(Step {
                    ticket,
                    project,
                    catalog,
                    member: again,
                    stage,
                    worktree_path,
                    handoffs,
                    blocking: &[],
                    blocked_by: Blocked::Review,
                    git: self.git_for(catalog, &reviewer.role),
                    appendix: &appendix,
                })
                .await;
            if !matches!(step, StepOutcome::Done) {
                return step;
            }
            stage += 1;
            // From here the verdict is one we produced, so it is published.
            fresh = true;
        }
    }

    /// The facts under every description, written by us rather than asked of
    /// an agent: they must be right, and only the daemon knows them all.
    async fn pr_footer(&self, ticket: &Ticket, branch: &str) -> String {
        let verdict = match self.last_verdict(ticket.id).await {
            Some((round, verdict)) => format!("relecture {round} : {}", verdict.label_fr()),
            None => "sans verdict enregistré".to_string(),
        };
        let cost = match self.ledger.ticket_cost(ticket.id).await {
            Ok(cost) => format!(
                "{} tokens · {} (indicatif)",
                orchestra_core::pricing::fmt_tokens(cost.tokens.total()),
                cost.cost_usd
                    .map(orchestra_core::pricing::fmt_usd)
                    .unwrap_or_else(|| "coût inconnu".into())
            ),
            Err(_) => String::new(),
        };
        format!(
            "---\n\nTicket #{} — {} · branche `{branch}` · {verdict} · {cost}\n\n\
             Ouverte par Orchestra.",
            ticket.number, ticket.title
        )
    }

    /// The last verdict a relecture rendered, and which round it was.
    async fn last_verdict(
        &self,
        ticket_id: TicketId,
    ) -> Option<(u32, orchestra_core::review::Verdict)> {
        let filter = orchestra_core::events::EventFilter {
            ticket_id: Some(ticket_id),
            tags: vec![orchestra_core::events::EventTag::ReviewVerdict],
            ..Default::default()
        };
        self.store
            .recent_events(filter, 1)
            .await
            .ok()?
            .into_iter()
            .find_map(|e| match e.kind {
                EventKind::ReviewVerdict { round, verdict, .. } => Some((round, verdict)),
                _ => None,
            })
    }

    /// How many blocking verdicts this ticket already collected: one per
    /// correction round spent.
    async fn blocking_verdicts(&self, ticket_id: TicketId) -> u32 {
        let filter = orchestra_core::events::EventFilter {
            ticket_id: Some(ticket_id),
            tags: vec![orchestra_core::events::EventTag::ReviewVerdict],
            ..Default::default()
        };
        self.store
            .recent_events(filter, 50)
            .await
            .unwrap_or_default()
            .iter()
            .filter(|e| {
                matches!(&e.kind, EventKind::ReviewVerdict { verdict, .. } if !verdict.is_ready())
            })
            .count() as u32
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
            blocking,
            blocked_by,
            git,
            appendix,
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
        let label = crate::zellij::pane_name(&project.name, &member.role, ticket.number);

        let prompt = build_prompt(
            ticket,
            member,
            worktree_path,
            handoffs,
            blocking,
            blocked_by,
        );
        let rules = crate::rules::book(&self.conventions_dir, Some(project))
            .prompt_section(&role.name, self.cfg.integration.mode.is_pr());
        let prompt_file = write_prompt_file(
            &self.cache_dir,
            &role.name,
            &format!("{}\n\n{FOOTER}{rules}{appendix}", role.system_prompt),
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
                    role,
                    &prompt_file,
                    &turn_prompt,
                    worktree_path,
                    model.clone(),
                    effort,
                    budget,
                    resume,
                    git,
                    &label,
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
        role: &RoleDefinition,
        prompt_file: &Path,
        prompt: &str,
        worktree_path: &Path,
        model: Option<String>,
        effort: Effort,
        budget: Option<f64>,
        resume: Option<Uuid>,
        git: GitPolicy,
        // `[projet] rôle #12`: the session's name, and its pane's.
        label: &str,
    ) -> Result<AgentOutcome> {
        let mut cmd = ClaudeCommand::new(&self.cfg.daemon.claude_bin, worktree_path, prompt);
        cmd.session_id = Some(agent.session_id);
        cmd.resume = resume;
        cmd.name = Some(label.to_string());
        cmd.model = model;
        cmd.effort = Some(effort);
        cmd.permission_mode = Some("bypassPermissions".into());
        cmd.append_system_prompt_file = Some(prompt_file.to_path_buf());
        cmd.allowed_tools = role.allowed_tools.clone();
        cmd.disallowed_tools = role
            .disallowed_tools
            .iter()
            .cloned()
            .chain(hooks::always_disallowed(git))
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
            git,
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

        // The pane comes after the agent is on record: what runs in it is
        // `orchestra tail <id>`, which asks the daemon for that very agent.
        if self.cfg.zellij.auto_pane || self.panes_wanted.lock().await.contains(&agent.ticket_id) {
            self.open_pane(agent, label, worktree_path, None).await;
        }

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
    /// What git a role gets: what its file declares (`git:`), and without a
    /// word there, the open policy for the configured integrator alone
    /// ([`GitPolicy::for_role`]). Read from the catalog at every step, so an
    /// integrator an orchestrator put in a team keeps its git, and a right the
    /// user just gave a role applies to the next agent launched.
    fn git_for(&self, catalog: &Catalog, role: &str) -> GitPolicy {
        GitPolicy::for_role(
            catalog.get(role).and_then(|r| r.git),
            role,
            &self.cfg.integration.role,
        )
    }

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
        // A pane that still says « backend #12 » an hour after backend stopped
        // is a screen that lies by omission. The mark goes on as the agent
        // ends, wherever it ended from.
        if !status.is_active() {
            self.mark_pane(agent).await;
        }
    }

    /// Open a pane following this agent, on demand rather than on spawn.
    pub async fn open_tail_pane(&self, agent: Agent, worktree: &Path) -> Option<String> {
        let mut agent = agent;
        let label = self.pane_label(&agent).await;
        self.open_pane(&mut agent, &label, worktree, None).await
    }

    /// Hand an agent's session back to the user, in a pane they drive.
    ///
    /// The status change is the important half: from here on the daemon does
    /// not steer, cancel or count turns for it. The watcher still counts its
    /// tokens, because the session id is the same one it was already tailing.
    pub async fn hand_over(
        &self,
        agent: Agent,
        worktree: &Path,
        claude_bin: &str,
    ) -> Option<String> {
        let mut agent = agent;
        // Plain here: the mark is put on by `set_status` below, and putting it
        // on twice is how a pane ends up called « ☰ ☰ … ».
        let label = self.pane_label(&agent).await;
        let command = vec![
            claude_bin.to_string(),
            "--resume".to_string(),
            agent.session_id.to_string(),
        ];
        let pane_id = self
            .open_pane(&mut agent, &label, worktree, Some(command))
            .await?;
        self.set_status(&mut agent, AgentStatus::Manual, None).await;
        Some(pane_id)
    }

    /// `[projet] rôle #12`, rebuilt from the store for a pane opened later.
    async fn pane_label(&self, agent: &Agent) -> String {
        let number = match self.store.ticket(agent.ticket_id).await {
            Ok(Some(ticket)) => ticket.number,
            _ => 0,
        };
        let project = match self.store.project(agent.project_id).await {
            Ok(Some(project)) => project.name,
            _ => "orchestra".to_string(),
        };
        crate::zellij::pane_name(&project, &agent.role, number)
    }

    /// Open the pane that shows this agent, and remember its id.
    ///
    /// Best-effort throughout: no pane, no zellij, no matter. `command` is what
    /// runs in it, and defaults to following the agent's log.
    async fn open_pane(
        &self,
        agent: &mut Agent,
        label: &str,
        cwd: &Path,
        command: Option<Vec<String>>,
    ) -> Option<String> {
        if !crate::zellij::inside() {
            return None;
        }
        let command = match command {
            Some(c) => c,
            // Our own binary rather than whatever `orchestra` a PATH resolves
            // to: the pane must follow the daemon it belongs to.
            None => vec![
                std::env::current_exe().ok()?.display().to_string(),
                "tail".into(),
                agent.id.to_string(),
            ],
        };
        let pane_id = crate::zellij::new_pane(label, cwd, &command).await?;
        agent.pane_id = Some(pane_id.clone());
        if let Err(e) = self.store.update_agent(agent.clone()).await {
            tracing::warn!("pane non rattaché à l'agent : {e:#}");
        }
        self.emit(
            agent,
            EventKind::PaneOpened {
                pane_id: pane_id.clone(),
            },
        )
        .await;
        Some(pane_id)
    }

    /// Put the outcome in the pane's name, so a wall of them reads at a glance.
    async fn mark_pane(&self, agent: &Agent) {
        let (Some(pane_id), true) = (agent.pane_id.as_deref(), crate::zellij::inside()) else {
            return;
        };
        let Ok(Some(ticket)) = self.store.ticket(agent.ticket_id).await else {
            return;
        };
        let Ok(Some(project)) = self.store.project(agent.project_id).await else {
            return;
        };
        let base = crate::zellij::pane_name(&project.name, &agent.role, ticket.number);
        crate::zellij::rename(pane_id, &crate::zellij::finished_name(&base, agent.status)).await;
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
/// Why a role is being sent back to work.
///
/// The two are not the same thing to the agent that reads them: a verdict is
/// someone's reading of the branch, a red check is the branch refusing to
/// build. Naming the source is what lets it answer the right one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    Review,
    Checks,
    Rules,
}

impl Blocked {
    fn title_fr(self) -> &'static str {
        match self {
            Blocked::Review => "Ce que la relecture bloque",
            Blocked::Checks => "Ce que les vérifications du dépôt refusent",
            Blocked::Rules => "Ce que les conventions de l'équipe refusent",
        }
    }

    fn order_fr(self) -> &'static str {
        match self {
            Blocked::Review => "Corrige exactement ces points.",
            Blocked::Checks => "Fais repasser cette commande au vert.",
            Blocked::Rules => {
                "Mets la branche en conformité : reformule les commits concernés \
                 (`git rebase`, sans rien changer d'autre), complète PR.md s'il le faut."
            }
        }
    }
}

struct RoleRun<'a> {
    ticket: &'a Ticket,
    project: &'a Project,
    role: &'a RoleDefinition,
    member: &'a orchestra_core::model::TeamMember,
    stage: u32,
    worktree_path: &'a Path,
    handoffs: &'a [(String, String)],
    /// What blocked, when this run is a correction round.
    blocking: &'a [String],
    /// Where that came from, so the prompt names it truthfully.
    blocked_by: Blocked,
    /// What this run may do with git. Only the integrator gets more than the
    /// confined policy.
    git: GitPolicy,
    /// Added to the role's own instructions for this run alone.
    appendix: &'a str,
}

/// One step of the walk: a member to run, and the ticket's memory to fold its
/// handoff into.
struct Step<'a> {
    ticket: &'a Ticket,
    project: &'a Project,
    catalog: &'a Catalog,
    /// Owned, because a correction round runs a member whose objective was
    /// rewritten from the verdict rather than one the user accepted.
    member: orchestra_core::model::TeamMember,
    stage: u32,
    worktree_path: &'a Path,
    handoffs: &'a mut Vec<(String, String)>,
    blocking: &'a [String],
    blocked_by: Blocked,
    git: GitPolicy,
    appendix: &'a str,
}

/// What one step left the ticket in.
enum StepOutcome {
    Done,
    Cancelled,
    Failed,
}

/// The blocking points this role has to answer for. A role the relecture named
/// gets its own lines; the one that catches the unattributed remarks gets all
/// of them.
fn blocking_for(
    review: &orchestra_core::review::Review,
    role: &str,
    named: &[String],
) -> Vec<String> {
    let mine: Vec<String> = review
        .changes
        .iter()
        .filter(|c| c.role.as_deref().is_some_and(|r| r == role))
        .map(|c| c.detail.clone())
        .collect();
    if mine.is_empty() || named.len() == 1 {
        review.blocking_lines()
    } else {
        mine
    }
}

/// The handoff of the last agent of this role that finished, if it left one.
///
/// This is what makes a relaunch a resumption: the work is in the branch and
/// what the role said is in the store, so the stage can be taken as done.
fn finished_handoff(prior: &[Agent], role: &str) -> Option<String> {
    prior
        .iter()
        .filter(|a| a.role == role && a.status == AgentStatus::Done)
        .max_by_key(|a| a.started_at)
        .and_then(|a| a.handoff.clone())
        .filter(|h| !h.trim().is_empty())
}

/// Roles that finished a run started after the last relecture: on a resumed
/// ticket, they have already answered the verdict that is about to be read.
fn corrected_since_review(prior: &[Agent], review_role: &str) -> Vec<String> {
    let Some(last_review) = prior
        .iter()
        .filter(|a| a.role == review_role && a.status == AgentStatus::Done)
        .filter_map(|a| a.started_at)
        .max()
    else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for agent in prior.iter().filter(|a| {
        a.role != review_role
            && a.status == AgentStatus::Done
            && a.started_at.is_some_and(|s| s > last_review)
    }) {
        if !out.contains(&agent.role) {
            out.push(agent.role.clone());
        }
    }
    out
}

/// What the reviewer is told about the gate that has just run for it.
///
/// Its role asks it to find how the project verifies itself and run it. That
/// has already happened, twenty seconds ago, by something that cannot be
/// mistaken about the result — so it is handed over rather than paid for a
/// second time at agent prices. With the command named, because a reviewer
/// that cannot see what ran has to take our word for it, and this whole gate
/// exists so that nobody has to take anybody's word for it.
fn checks_appendix(outcome: &orchestra_core::checks::ChecksOutcome) -> String {
    if outcome.runs.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n## Les vérifications du dépôt ont déjà tourné\n\nOrchestra les a lancées \
         dans ce worktree juste avant toi :\n\n",
    );
    for run in &outcome.runs {
        out.push_str(&format!("- {}\n", run.label_fr()));
    }
    out.push_str(
        "\nNe les relance pas : ce résultat fait foi, et le temps que tu y passerais \
         est mieux employé à lire le code. S'il te semble qu'une vérification manque \
         au dépôt, dis-le dans ton rapport plutôt que de la lancer à la main.\n",
    );
    out
}

/// The last role that actually wrote something, which is the one to hand a
/// broken build to: it has the freshest context, and a compiler error is
/// almost always about what was just written.
fn last_worker(handoffs: &[(String, String)], workers: &[String]) -> Option<String> {
    handoffs
        .iter()
        .rev()
        .map(|(role, _)| role)
        .find(|role| workers.contains(role))
        .cloned()
        .or_else(|| workers.last().cloned())
}

/// What a role is asked to do when it comes back, after a verdict or a red
/// check.
fn correction_objective(original: &str, blocking: &[String], blocked_by: Blocked) -> String {
    let list = blocking
        .iter()
        .map(|l| format!("- {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let head = match blocked_by {
        Blocked::Review => "Lever ce que la relecture a bloqué, et rien d'autre :",
        Blocked::Checks => "Réparer ce que la vérification du dépôt refuse, et rien d'autre :",
        Blocked::Rules => {
            "Mettre la branche en conformité avec les conventions de l'équipe, et rien d'autre :"
        }
    };
    format!("{head}\n{list}\n\n         (ton objectif initial était : {original})")
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

enum AgentOutcome {
    Done { handoff: String },
    Failed { reason: String },
    Cancelled,
}

/// Where the integrator leaves the description it wrote.
const PR_BODY_FILE: &str = "PR.md";

/// What the integrator wrote following the template, if it wrote anything.
///
/// An empty file counts as nothing: a run that created it and gave up should
/// not open a request with a blank description.
fn written_pr_body(worktree: &Path) -> Option<String> {
    std::fs::read_to_string(worktree.join(PR_BODY_FILE))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// The description used when the integrator left none.
///
/// The brief says what was asked, the last handoff says what was delivered:
/// crude next to a written one, but never empty, and the sections line up with
/// the template so a request always reads the same way.
fn assembled_pr_body(ticket: &Ticket, handoffs: &[(String, String)]) -> String {
    let mut out = String::from("## Pourquoi\n\n");
    out.push_str(ticket.brief.trim());
    if let Some((role, last)) = handoffs.last() {
        out.push_str(&format!(
            "\n\n## Ce que « {role} » a livré\n\n{}",
            orchestra_core::claude::stream::truncate(last.trim(), 20_000)
        ));
    }
    out
}

/// What the integrator is told to do, in one sentence it can act on.
fn integration_objective(
    default_branch: &str,
    branch: &str,
    push: bool,
    mode: orchestra_core::config::IntegrationMode,
) -> String {
    let ending = if mode.is_pr() {
        " Quand tu auras fini, la branche sera poussée et une pull request ouverte          dessus : laisse-la dans un état qu'on peut relire, et dis dans ton résumé ce          qu'il faut savoir pour la valider."
    } else if push {
        " Pousse ensuite la branche sur son remote."
    } else {
        " Ne pousse rien : ce dépôt s'intègre en local."
    };
    format!(
        "Rendre « {branch} » fusionnable en avance rapide dans « {default_branch} » : \
         rapatrier {default_branch} dans ta branche, régler les conflits, rejouer les \
         vérifications du projet et laisser un historique propre.{ending} La fusion \
         finale ne t'appartient pas.",
    )
}

/// The first message an agent receives: the ticket, its own objective, what
/// the team did before it, and the rules of the worktree.
pub fn build_prompt(
    ticket: &Ticket,
    member: &orchestra_core::model::TeamMember,
    worktree: &std::path::Path,
    handoffs: &[(String, String)],
    blocking: &[String],
    blocked_by: Blocked,
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

    if !blocking.is_empty() {
        // Repeated even though the objective already carries them: an agent
        // that has just read three handoffs needs the blocking list where it
        // cannot be missed.
        out.push_str(&format!("\n## {}\n\n", blocked_by.title_fr()));
        for line in blocking {
            out.push_str(&format!("- {line}\n"));
        }
        out.push_str(&format!(
            "\n{} Ne refais pas le reste, n'en profite pas pour autre chose, et si tu \
             penses qu'un point n'en est pas un, dis-le dans ton résumé au lieu de \
             l'ignorer en silence.\n",
            blocked_by.order_fr()
        ));
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

    /// An agent row as the store hands it back.
    fn agent(role: &str, status: AgentStatus, minute: i64, handoff: Option<&str>) -> Agent {
        Agent {
            id: Uuid::new_v4(),
            ticket_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            role: role.into(),
            objective: "o".into(),
            stage: 0,
            session_id: Uuid::new_v4(),
            model: "sonnet".into(),
            effort: Effort::High,
            max_budget_usd: None,
            status,
            exit_reason: None,
            pid: None,
            pane_id: None,
            attempt: 1,
            handoff: handoff.map(str::to_string),
            started_at: Some(orchestra_core::now() + time::Duration::minutes(minute)),
            ended_at: None,
        }
    }

    #[test]
    fn a_relaunch_takes_finished_work_as_it_stands() {
        let prior = vec![
            agent("backend", AgentStatus::Done, 1, Some("fait : le cache")),
            agent("tests", AgentStatus::Crashed, 2, None),
        ];
        assert_eq!(
            finished_handoff(&prior, "backend").as_deref(),
            Some("fait : le cache")
        );
        // Interrompu, donc à refaire.
        assert!(finished_handoff(&prior, "tests").is_none());
        assert!(finished_handoff(&prior, "docs").is_none());

        // Un agent terminé sans rien dire ne vaut pas un passage de relais.
        let muet = vec![agent("docs", AgentStatus::Done, 1, Some("  "))];
        assert!(finished_handoff(&muet, "docs").is_none());

        // La dernière reprise d'un rôle fait foi.
        let deux = vec![
            agent("backend", AgentStatus::Done, 1, Some("premier jet")),
            agent("backend", AgentStatus::Done, 3, Some("après relecture")),
        ];
        assert_eq!(
            finished_handoff(&deux, "backend").as_deref(),
            Some("après relecture")
        );
    }

    #[test]
    fn a_correction_already_made_is_not_paid_for_twice() {
        // Le cas réel : la relecture bloque, le rôle corrige, le daemon
        // redémarre avant que la relecture suivante ait pu tourner.
        let prior = vec![
            agent("frontend", AgentStatus::Done, 1, Some("livré")),
            agent(
                "reviewer",
                AgentStatus::Done,
                2,
                Some("VERDICT: corrections"),
            ),
            agent("frontend", AgentStatus::Done, 3, Some("corrigé")),
            agent("reviewer", AgentStatus::Crashed, 4, None),
        ];
        assert_eq!(
            corrected_since_review(&prior, "reviewer"),
            vec!["frontend".to_string()],
            "le tour de correction est déjà dans la branche"
        );

        // Sans relecture, personne n'a de correction à son actif.
        let avant = vec![agent("frontend", AgentStatus::Done, 1, Some("livré"))];
        assert!(corrected_since_review(&avant, "reviewer").is_empty());

        // Et un rôle qui a fini avant la relecture n'a pas répondu au verdict.
        let apres = vec![
            agent("frontend", AgentStatus::Done, 1, Some("livré")),
            agent(
                "reviewer",
                AgentStatus::Done,
                2,
                Some("VERDICT: corrections"),
            ),
        ];
        assert!(corrected_since_review(&apres, "reviewer").is_empty());
    }

    #[test]
    fn the_prompt_carries_the_ticket_the_objective_and_the_boundary() {
        let prompt = build_prompt(
            &ticket(),
            &member("backend"),
            Path::new("/home/u/wt/7-cache"),
            &[],
            &[],
            Blocked::Review,
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
            &[],
            Blocked::Review,
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
    fn the_description_written_by_the_integrator_wins() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            written_pr_body(dir.path()).is_none(),
            "pas de fichier, pas de description"
        );

        std::fs::write(dir.path().join(PR_BODY_FILE), "   \n\n  ").unwrap();
        assert!(
            written_pr_body(dir.path()).is_none(),
            "un fichier vide ne vaut pas une description"
        );

        std::fs::write(
            dir.path().join(PR_BODY_FILE),
            "\n## Ce que ça change\n\nLe thème clair se garde.\n\n",
        )
        .unwrap();
        let body = written_pr_body(dir.path()).unwrap();
        assert!(body.starts_with("## Ce que ça change"), "{body}");
        assert!(body.ends_with("se garde."), "les bords sont nettoyés");
    }

    #[test]
    fn the_assembled_description_keeps_the_same_shape() {
        // Sans fichier écrit, la requête doit quand même se lire comme les
        // autres : mêmes titres de section, dans le même ordre.
        let handoffs = vec![("integrator".to_string(), "fusion propre".to_string())];
        let body = assembled_pr_body(&ticket(), &handoffs);
        assert!(body.starts_with("## Pourquoi"), "{body}");
        assert!(body.contains("On veut un cache"), "{body}");
        assert!(body.contains("## Ce que « integrator » a livré"), "{body}");
        assert!(
            !body.contains("Ouverte par Orchestra"),
            "le pied de page est ajouté par le daemon, pas ici"
        );
    }

    #[test]
    fn a_correction_round_says_what_to_fix_and_what_not_to_touch() {
        let blocking = vec![
            "store/rows.rs:88 boucle quand offset dépasse le total".to_string(),
            "rien ne couvre la liste vide".to_string(),
        ];
        let mut m = member("backend");
        m.objective = correction_objective(&m.objective, &blocking, Blocked::Review);
        let prompt = build_prompt(
            &ticket(),
            &m,
            Path::new("/home/u/wt/7-cache"),
            &[("reviewer".to_string(), "VERDICT: corrections".to_string())],
            &blocking,
            Blocked::Review,
        );
        assert!(prompt.contains("Ce que la relecture bloque"));
        assert!(prompt.contains("boucle quand offset"));
        assert!(prompt.contains("liste vide"));
        assert!(
            prompt.contains("Ne refais pas le reste"),
            "un tour de correction n'est pas une réécriture : {prompt}"
        );
        assert!(
            m.objective.contains("écrire le cache dans store/rows.rs"),
            "l'objectif initial reste lisible"
        );
    }

    #[test]
    fn each_role_gets_its_own_blocking_points() {
        let review = orchestra_core::review::parse_review(
            "VERDICT: corrections\n- backend: la boucle\n- tests: le cas vide",
        )
        .unwrap();
        let named = vec!["backend".to_string(), "tests".to_string()];
        assert_eq!(blocking_for(&review, "backend", &named), vec!["la boucle"]);
        assert_eq!(blocking_for(&review, "tests", &named), vec!["le cas vide"]);

        // One role alone carries the whole list, including what named nobody.
        let review =
            orchestra_core::review::parse_review("VERDICT: corrections\n- ça casse au bord")
                .unwrap();
        let alone = vec!["backend".to_string()];
        assert_eq!(
            blocking_for(&review, "backend", &alone),
            vec!["ça casse au bord"]
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
        let sup = Supervisor::new(store, bus, ledger, cfg, dir.path().to_path_buf(), dir.path().join("conventions"));

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
        let err = sup
            .launch(ticket(), project, catalog, false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("équipe acceptée"), "{err}");
    }

    /// A supervisor over an in-memory store, plus a project and a ticket the
    /// events can hang off.
    async fn gated(
        commands: Vec<String>,
        max_rounds: u32,
    ) -> (Supervisor, Ticket, Project, tempfile::TempDir) {
        let store = Store::open_memory().unwrap();
        let bus = EventBus::new(store.clone());
        let mut cfg = Config::default();
        cfg.checks.commands = commands;
        cfg.checks.max_rounds = max_rounds;
        cfg.checks.timeout_secs = 20;
        let cfg = Arc::new(cfg);
        let ledger = UsageLedger::new(store.clone(), &cfg);
        let dir = tempfile::tempdir().unwrap();
        let project = Project {
            id: Uuid::new_v4(),
            name: "p".into(),
            path: dir.path().to_path_buf(),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        };
        let mut t = ticket();
        t.project_id = project.id;
        store.insert_project(project.clone()).await.unwrap();
        store.insert_ticket(t.clone()).await.unwrap();
        let sup = Supervisor::new(store, bus, ledger, cfg, dir.path().to_path_buf(), dir.path().join("conventions"));
        (sup, t, project, dir)
    }

    fn team_of(roles: &[&str]) -> Team {
        Team {
            members: roles.iter().map(|r| member(r)).collect(),
            stages: roles.iter().map(|r| vec![r.to_string()]).collect(),
        }
    }

    #[tokio::test]
    async fn a_green_gate_lets_the_relecture_happen() {
        let (sup, t, project, dir) = gated(vec!["true".into()], 2).await;
        let team = team_of(&["backend", "reviewer"]);
        let catalog = Catalog::load(dir.path(), None);
        let mut handoffs = vec![("backend".to_string(), "fait".to_string())];
        let mut stage = 1;
        let out = sup
            .checks_gate(
                &t,
                &project,
                &team,
                &catalog,
                dir.path(),
                &mut handoffs,
                &mut stage,
            )
            .await;
        assert!(matches!(out, StepOutcome::Done));

        let outcome = sup.last_checks(t.id).await.expect("une passe a eu lieu");
        assert!(outcome.passed());
        assert_eq!(outcome.round, 1);
        assert_eq!(outcome.runs[0].command, "true");
    }

    #[tokio::test]
    async fn a_red_gate_fails_the_ticket_rather_than_paying_a_relecture() {
        // `max_rounds = 0` : aucune réparation n'est due, donc rien ne tente de
        // lancer un agent — ce que ce test mesure, c'est la décision.
        let (sup, t, project, dir) = gated(vec!["false".into()], 0).await;
        let team = team_of(&["backend", "reviewer"]);
        let catalog = Catalog::load(dir.path(), None);
        let mut handoffs = vec![("backend".to_string(), "fait".to_string())];
        let mut stage = 1;
        let out = sup
            .checks_gate(
                &t,
                &project,
                &team,
                &catalog,
                dir.path(),
                &mut handoffs,
                &mut stage,
            )
            .await;
        assert!(
            matches!(out, StepOutcome::Failed),
            "un rouge arrête le ticket"
        );

        let outcome = sup.last_checks(t.id).await.unwrap();
        assert!(!outcome.passed());
        assert_eq!(outcome.failed().unwrap().command, "false");
    }

    #[tokio::test]
    async fn a_project_with_no_check_is_not_held_back_by_one() {
        let (sup, t, project, dir) = gated(vec![], 2).await;
        let team = team_of(&["backend", "reviewer"]);
        let catalog = Catalog::load(dir.path(), None);
        let mut handoffs = Vec::new();
        let mut stage = 1;
        let out = sup
            .checks_gate(
                &t,
                &project,
                &team,
                &catalog,
                dir.path(),
                &mut handoffs,
                &mut stage,
            )
            .await;
        assert!(matches!(out, StepOutcome::Done));
        assert!(sup.last_checks(t.id).await.is_none(), "rien n'a tourné");
    }

    #[tokio::test]
    async fn a_first_command_that_refuses_stops_the_pass() {
        // Le reste ne rapporterait que sur un arbre dont on sait déjà qu'il ne
        // tient pas, et chaque commande coûte du temps réel.
        let (sup, t, project, dir) = gated(vec!["false".into(), "true".into()], 0).await;
        let team = team_of(&["backend", "reviewer"]);
        let catalog = Catalog::load(dir.path(), None);
        let mut handoffs = vec![("backend".to_string(), "fait".to_string())];
        let mut stage = 1;
        let _ = sup
            .checks_gate(
                &t,
                &project,
                &team,
                &catalog,
                dir.path(),
                &mut handoffs,
                &mut stage,
            )
            .await;
        let outcome = sup.last_checks(t.id).await.unwrap();
        assert_eq!(outcome.runs.len(), 1, "la seconde n'a pas été lancée");
    }

    #[tokio::test]
    async fn the_relecture_is_handed_the_result_instead_of_running_it_again() {
        let (sup, t, project, dir) = gated(vec!["true".into()], 2).await;
        let team = team_of(&["backend", "reviewer"]);
        let catalog = Catalog::load(dir.path(), None);
        let mut handoffs = Vec::new();
        let mut stage = 1;
        sup.checks_gate(
            &t,
            &project,
            &team,
            &catalog,
            dir.path(),
            &mut handoffs,
            &mut stage,
        )
        .await;
        let appendix = checks_appendix(&sup.last_checks(t.id).await.unwrap());
        assert!(appendix.contains("Ne les relance pas"), "{appendix}");
        assert!(
            appendix.contains("true"),
            "la commande est nommée : {appendix}"
        );
    }

    #[test]
    fn a_broken_build_goes_to_whoever_wrote_last() {
        let workers = vec!["architect".to_string(), "backend".to_string()];
        let handoffs = vec![
            ("architect".to_string(), "plan".to_string()),
            ("backend".to_string(), "code".to_string()),
            ("reviewer".to_string(), "VERDICT: prêt".to_string()),
        ];
        assert_eq!(last_worker(&handoffs, &workers).as_deref(), Some("backend"));
        // Personne n'a encore rendu la main : le dernier rôle de l'équipe.
        assert_eq!(last_worker(&[], &workers).as_deref(), Some("backend"));
    }

    /// A repository at `dir` whose branch `t` carries one commit per message.
    fn repo_with_branch(dir: &Path, messages: &[&str]) {
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} : {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        git(&["commit", "-q", "--allow-empty", "-m", "départ"]);
        git(&["checkout", "-q", "-b", "t"]);
        for m in messages {
            git(&["commit", "-q", "--allow-empty", "-m", m]);
        }
    }

    async fn rules_checked(sup: &Supervisor, t: &Ticket) -> Vec<Vec<orchestra_core::conventions::Violation>> {
        let filter = orchestra_core::events::EventFilter {
            ticket_id: Some(t.id),
            tags: vec![orchestra_core::events::EventTag::RulesChecked],
            ..Default::default()
        };
        sup.store
            .recent_events(filter, 10)
            .await
            .unwrap()
            .into_iter()
            .filter_map(|e| match e.kind {
                EventKind::RulesChecked { violations, .. } => Some(violations),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_branch_that_breaks_a_measured_convention_is_not_fused() {
        // `max_rounds = 0` : pas de tour de mise en conformité, donc aucun
        // agent n'est lancé — ce qui est testé, c'est la décision.
        let (sup, t, project, dir) = gated(vec![], 0).await;
        repo_with_branch(dir.path(), &["[backend] ajoute le cache", "wip"]);
        let catalog = Catalog::load(dir.path(), None);
        let mut handoffs = Vec::new();
        let mut stage = 3;
        let ok = sup
            .rules_gate(
                &t,
                &project,
                &catalog,
                &member("integrator"),
                "t",
                dir.path(),
                &mut handoffs,
                &mut stage,
            )
            .await;
        assert!(!ok, "un commit « wip » bloque la fusion");
        let passes = rules_checked(&sup, &t).await;
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].len(), 1, "{:?}", passes[0]);
        assert_eq!(passes[0][0].rule, "commits");
        assert!(passes[0][0].detail.contains("« wip »"));
    }

    #[tokio::test]
    async fn a_conforming_branch_goes_on_and_merges_are_not_judged() {
        let (sup, t, project, dir) = gated(vec![], 0).await;
        repo_with_branch(dir.path(), &["[backend] ajoute le cache", "[tests] couvre le cas vide"]);
        // A merge of the default branch into the ticket's: git's wording.
        let git = |args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(dir.path()).args(args).output().unwrap()
        };
        git(&["checkout", "-q", "main"]);
        git(&["commit", "-q", "--allow-empty", "-m", "ailleurs"]);
        git(&["checkout", "-q", "t"]);
        assert!(git(&["merge", "-q", "--no-edit", "main"]).status.success());

        let catalog = Catalog::load(dir.path(), None);
        let mut handoffs = Vec::new();
        let mut stage = 3;
        let ok = sup
            .rules_gate(
                &t,
                &project,
                &catalog,
                &member("integrator"),
                "t",
                dir.path(),
                &mut handoffs,
                &mut stage,
            )
            .await;
        assert!(ok);
        assert_eq!(rules_checked(&sup, &t).await, vec![vec![]]);
    }

    #[tokio::test]
    async fn a_rejected_convention_no_longer_gates_anything() {
        let (sup, t, project, dir) = gated(vec![], 0).await;
        repo_with_branch(dir.path(), &["wip"]);
        // The project switches the shipped convention off by overriding it.
        let own = dir.path().join(".orchestra/conventions");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(
            own.join("commits.md"),
            "---\ntitle: Commits libres\nstatus: rejected\n---\nPas de format imposé.\n",
        )
        .unwrap();
        let catalog = Catalog::load(dir.path(), None);
        let mut handoffs = Vec::new();
        let mut stage = 3;
        assert!(
            sup.rules_gate(&t, &project, &catalog, &member("integrator"), "t", dir.path(), &mut handoffs, &mut stage)
                .await
        );
        assert!(rules_checked(&sup, &t).await.is_empty(), "rien à vérifier, rien de publié");
    }

    #[tokio::test]
    async fn an_agent_proposal_is_written_pending_and_announced() {
        let (sup, t, project, dir) = gated(vec![], 0).await;
        let handoff = "Fait.\n\nPROPOSITION: convention\ntitre: Paginer les listes\nrôles: backend\n\
                       Toute liste prend limit et offset.\nFIN PROPOSITION\n";
        sup.record_proposals(&t, &project, "reviewer", handoff).await;

        let path = dir.path().join(".orchestra/conventions/paginer-les-listes.md");
        let src = std::fs::read_to_string(&path).expect("la proposition est écrite");
        assert!(src.contains("status: proposed"), "{src}");
        assert!(src.contains("reviewer, ticket #7"), "{src}");

        // Written, but never handed to an agent before it is accepted.
        let book = crate::rules::book(&dir.path().join("conventions"), Some(&project));
        assert_eq!(book.pending(), 1);
        assert!(!book.prompt_section("backend", false).contains("Paginer"));

        let filter = orchestra_core::events::EventFilter {
            ticket_id: Some(t.id),
            tags: vec![orchestra_core::events::EventTag::RuleProposed],
            ..Default::default()
        };
        let events = sup.store.recent_events(filter, 5).await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn steering_an_agent_that_is_not_running_says_so() {
        let store = Store::open_memory().unwrap();
        let bus = EventBus::new(store.clone());
        let cfg = Arc::new(Config::default());
        let ledger = UsageLedger::new(store.clone(), &cfg);
        let dir = tempfile::tempdir().unwrap();
        let sup = Supervisor::new(store, bus, ledger, cfg, dir.path().to_path_buf(), dir.path().join("conventions"));

        let err = sup
            .steer(Uuid::new_v4(), "vas-y".into(), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("ne tourne pas"), "{err}");
        assert!(sup.cancel_agent(Uuid::new_v4()).await.is_err());
        // Cancelling an unknown ticket is harmless.
        assert!(sup.cancel_ticket(Uuid::new_v4()).await.is_ok());
    }

    #[test]
    fn the_integrator_keeps_its_git_wherever_it_runs() {
        let store = Store::open_memory().unwrap();
        let bus = EventBus::new(store.clone());
        let cfg = Arc::new(Config::default());
        let ledger = UsageLedger::new(store.clone(), &cfg);
        let dir = tempfile::tempdir().unwrap();
        let sup = Supervisor::new(store, bus, ledger, cfg.clone(), dir.path().to_path_buf(), dir.path().join("conventions"));
        // An installed catalog written before the field existed: the
        // integrator keeps its git by name, the others stay confined.
        let empty = Catalog::load(dir.path(), None);
        assert_eq!(sup.git_for(&empty, &cfg.integration.role), GitPolicy::Full);
        for role in ["frontend", "backend", "reviewer"] {
            assert_eq!(sup.git_for(&empty, role), GitPolicy::Confined, "{role}");
        }

        // Declared in the file, the right is the user's: given to one role,
        // taken from the integrator.
        let roles = dir.path().join("roles");
        std::fs::create_dir_all(&roles).unwrap();
        let with = |name: &str, git: GitPolicy| {
            let src = orchestra_core::roles::with_git(&orchestra_core::roles::skeleton(name), git)
                .unwrap();
            std::fs::write(roles.join(format!("{name}.md")), src).unwrap();
        };
        with("backend", GitPolicy::Full);
        with(&cfg.integration.role, GitPolicy::Confined);
        let declared = Catalog::load(&roles, None);
        assert_eq!(sup.git_for(&declared, "backend"), GitPolicy::Full);
        assert_eq!(
            sup.git_for(&declared, &cfg.integration.role),
            GitPolicy::Confined
        );
    }

    fn git_in(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} : {}", String::from_utf8_lossy(&out.stderr));
    }

    /// The fusion refused on a dirty main repository, then retried once it is
    /// clean: the second time needs no integrator, the branch is as it was.
    #[tokio::test]
    async fn a_merge_refused_at_the_door_is_retried_without_an_agent() {
        let store = Store::open_memory().unwrap();
        let bus = EventBus::new(store.clone());
        let mut cfg = Config::default();
        // Nothing here may start a real agent: if the fast path were missed,
        // this binary fails the integrator instead of calling a model.
        cfg.daemon.claude_bin = "false".into();
        let dir = tempfile::tempdir().unwrap();
        cfg.daemon.worktrees_dir = Some(dir.path().join("worktrees"));
        let cfg = Arc::new(cfg);
        let ledger = UsageLedger::new(store.clone(), &cfg);

        let repo = dir.path().join("depot");
        std::fs::create_dir_all(&repo).unwrap();
        git_in(&repo, &["init", "-q", "-b", "main"]);
        git_in(&repo, &["config", "user.email", "t@t"]);
        git_in(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("compose.yml"), "port: 5433").unwrap();
        git_in(&repo, &["add", "-A"]);
        git_in(&repo, &["commit", "-q", "-m", "départ"]);

        let project = Project {
            id: Uuid::new_v4(),
            name: "depot".into(),
            path: repo.clone(),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        };
        let mut t = ticket();
        t.project_id = project.id;
        t.status = TicketStatus::Review;
        store.insert_project(project.clone()).await.unwrap();
        store.insert_ticket(t.clone()).await.unwrap();
        let sup = Supervisor::new(store.clone(), bus, ledger, cfg, dir.path().to_path_buf(), dir.path().join("conventions"));

        let (branch, wt) = sup.ensure_worktree(&mut t, &project).await.unwrap();
        store.update_ticket(t.clone()).await.unwrap();
        std::fs::write(wt.join("favicon.svg"), "<svg/>").unwrap();
        git_in(&wt, &["add", "-A"]);
        git_in(&wt, &["commit", "-q", "-m", "[frontend] favicon"]);

        // The user is in the middle of something in the main repository.
        std::fs::write(repo.join("compose.yml"), "port: 5434").unwrap();
        sup.deliver(t.clone(), project.clone(), wt.clone(), branch.clone())
            .await
            .unwrap();
        let blocked = crate::integration::blocked_merge(&store, t.id)
            .await
            .expect("la fusion attend l'utilisateur");
        assert!(blocked.reason.contains("compose.yml"), "{}", blocked.reason);
        assert_eq!(
            Some(blocked.head.clone()),
            worktree::branch_head(&repo, &branch)
        );
        assert!(!repo.join("favicon.svg").exists(), "rien n'est fusionné");

        // Put away, and asked again.
        git_in(&repo, &["checkout", "--", "compose.yml"]);
        let catalog = Catalog::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/roles"),
            None,
        );
        sup.integrate(t.clone(), project.clone(), catalog).await.unwrap();
        for _ in 0..200 {
            if !sup.is_running(t.id).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(repo.join("favicon.svg").exists(), "la branche est dans main");
        let all = store
            .recent_events(
                orchestra_core::events::EventFilter::for_ticket(t.id),
                100,
            )
            .await
            .unwrap();
        assert!(
            !all.iter().any(|e| matches!(e.kind, EventKind::AgentSpawned { .. })),
            "aucun agent pour réessayer une porte"
        );
        assert!(all.iter().any(|e| matches!(e.kind, EventKind::TicketMerged { .. })));
        assert!(crate::integration::blocked_merge(&store, t.id).await.is_none());
        let back = store.ticket(t.id).await.unwrap().unwrap();
        assert_eq!(back.status, TicketStatus::Done);
    }
}
