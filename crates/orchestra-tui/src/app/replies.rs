//! What comes back from the daemon: replies, events, and the reads that ask for them.

use super::*;

impl App {
    /// Take the commands queued by the last update.
    pub fn take_outbox(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.outbox)
    }

    /// Ask the daemon for everything the current screen shows.
    pub fn refresh(&mut self) {
        self.outbox.push(Command::ListProjects);
        self.request_tickets();
        self.request_usage();
        self.outbox.push(Command::ListTodos);
        self.request_rules();
    }

    pub(super) fn request_tickets(&mut self) {
        // The epics' badge rides along with the board it sits on.
        self.request_epics();
        self.outbox.push(Command::ListTickets {
            project_id: self.selected_project().map(|p| p.id),
            status: None,
        });
    }

    /// Open the ticket of whichever agent is working.
    pub(super) fn adopt_live_agent(&mut self, agents: Vec<orchestra_core::protocol::AgentSummary>) {
        let Some(live) = agents
            .iter()
            .find(|a| a.agent.status.is_active())
            .or_else(|| agents.first())
        else {
            self.status = "aucun agent ne tourne en ce moment".into();
            return;
        };
        let already_open = self
            .ticket
            .as_ref()
            .is_some_and(|d| d.ticket.id == live.agent.ticket_id);
        if !already_open {
            self.outbox.push(Command::GetTicket {
                ticket_id: live.agent.ticket_id,
            });
        }
        self.pending_agent = Some(live.agent.id);
        self.status = format!("agent « {} »", live.agent.role);
    }

    /// Reload the open ticket when an event concerns it.
    pub(super) fn refresh_open_ticket(&mut self, ticket_id: Option<TicketId>) {
        let Some(open) = self.ticket.as_ref().map(|d| d.ticket.id) else {
            return;
        };
        if ticket_id.is_none_or(|id| id == open) {
            self.outbox.push(Command::GetTicket { ticket_id: open });
        }
    }

    pub(super) fn request_usage(&mut self) {
        for query in self.cost.queries() {
            self.outbox.push(Command::GetUsage { query });
        }
    }

    pub(super) fn on_reply(&mut self, reply: Reply) {
        match reply {
            Reply::Projects { projects } => {
                let previous = self.selected_project().map(|p| p.id);
                let first_load = !self.projects_loaded;
                self.projects_loaded = true;
                self.projects = projects
                    .into_iter()
                    .map(|p| ProjectRow {
                        id: p.id,
                        name: p.name,
                        path: p.path.display().to_string(),
                        discovered: p.kind == orchestra_core::model::ProjectKind::Discovered,
                    })
                    .collect();
                // Keep the highlight on the same project across refreshes,
                // or on "Tous les projets" if that is where it was, or if
                // the project it was on is simply gone. Only the very first
                // list, with nothing chosen yet, lands on the first project
                // instead of the aggregate — a single-project setup still
                // opens straight onto its own board.
                let fallback = if first_load && !self.projects.is_empty() {
                    1
                } else {
                    0
                };
                self.project_selected = previous
                    .and_then(|id| self.projects.iter().position(|p| p.id == id))
                    .map(|i| i + 1)
                    .unwrap_or(fallback)
                    .min(self.projects.len());
                if self.projects.is_empty() {
                    self.status =
                        "aucun projet — `:project add <chemin>` ou `orchestra project add`".into();
                }
            }
            Reply::Project { project } => {
                self.status = format!("projet « {} » ajouté", project.name);
                self.outbox.push(Command::ListProjects);
            }
            Reply::Tickets { tickets } => {
                let previous = self.selected_ticket_id();
                self.tickets = tickets;
                self.ticket_selected = previous
                    .and_then(|id| self.tickets.iter().position(|t| t.ticket.id == id))
                    .unwrap_or(0)
                    .min(self.tickets.len().saturating_sub(1));
            }
            Reply::Usage { rows, totals } => {
                // Two queries feed this screen and the reply does not name
                // which one it answers, so the trend is recognised by its key.
                let is_daily = rows.first().is_some_and(|r| r.keys.contains_key("day"));
                if is_daily {
                    let mut daily: Vec<(String, f64)> = rows
                        .into_iter()
                        .map(|r| {
                            (
                                r.keys.get("day").cloned().unwrap_or_default(),
                                r.cost_usd.unwrap_or(0.0),
                            )
                        })
                        .collect();
                    daily.sort_by(|a, b| a.0.cmp(&b.0));
                    self.cost.daily = daily;
                } else {
                    self.cost.rows = rows;
                    self.cost.totals = totals;
                    self.cost.selected = self
                        .cost
                        .selected
                        .min(self.cost.rows.len().saturating_sub(1));
                }
            }
            Reply::Epics { epics } => {
                self.epics = epics;
                self.epic_selected = self.epic_selected.min(self.epics.len().saturating_sub(1));
            }
            Reply::Epic { detail } => self.adopt_epic(detail),
            Reply::Diff { diff } => {
                self.status = format!(
                    "{} fichier(s) — j/k pour lire, q pour fermer",
                    diff.files.len()
                );
                self.diff_view = Some(DiffView {
                    diff,
                    scroll: 0,
                    height: std::cell::Cell::new(20),
                });
            }
            Reply::Ticket { detail } => {
                let previous = self.watched_agent_id();
                self.ticket = Some(detail);
                // A refresh used to clear this, so the indicator vanished a
                // second after the key was pressed while the orchestrator ran
                // for another minute. Only its own agent says when it is over.
                match self.orchestrator_agent().map(|a| a.agent.clone()) {
                    Some(a) if a.status.is_active() => {
                        self.planning = true;
                        self.planning_since.get_or_insert_with(|| {
                            a.started_at.unwrap_or_else(orchestra_core::now)
                        });
                    }
                    Some(a) if a.ended_at >= self.planning_since => self.stop_planning(),
                    _ => {}
                }
                // A ticket fetched to reach one particular agent.
                if let Some(wanted) = self.pending_agent.take() {
                    if let Some(index) = self
                        .ticket
                        .as_ref()
                        .and_then(|d| d.agents.iter().position(|a| a.agent.id == wanted))
                    {
                        self.agent_selected = index;
                        self.log.clear();
                        self.screen = Screen::Agent;
                        self.outbox.push(Command::Subscribe {
                            filter: orchestra_core::events::EventFilter::for_agent(wanted),
                            since_seq: None,
                            backlog: 500,
                        });
                        return;
                    }
                }
                // Keep watching the same agent across refreshes.
                if let Some(id) = previous {
                    if let Some(index) = self
                        .ticket
                        .as_ref()
                        .and_then(|d| d.agents.iter().position(|a| a.agent.id == id))
                    {
                        self.agent_selected = index;
                    }
                }
                let count = self.ticket.as_ref().map(|d| d.agents.len()).unwrap_or(0);
                self.agent_selected = self.agent_selected.min(count.saturating_sub(1));
                // The cursor follows the team: while watching, the screen moves
                // to whoever takes over; on the ticket, it is what scrolls the
                // list to the agent that works — one of a dozen rows, in a pane
                // that shows six. A row the user picked himself is never moved
                // out from under him there.
                let may_follow = match self.screen {
                    Screen::Ticket => !self.agent_pinned,
                    _ => !self.agent_hand_picked,
                };
                if may_follow
                    && !self
                        .watched_agent()
                        .is_some_and(|a| a.agent.status.is_active())
                {
                    let previous = self.watched_agent_id();
                    self.select_liveliest_agent();
                    if self.screen == Screen::Agent && self.watched_agent_id() != previous {
                        self.log.clear();
                        if let Some(id) = self.watched_agent_id() {
                            self.outbox.push(Command::Subscribe {
                                filter: orchestra_core::events::EventFilter::for_agent(id),
                                since_seq: None,
                                backlog: 500,
                            });
                        }
                    }
                }
            }
            Reply::Roles { roles, errors } => {
                // The cursor stays on what it was on, role or rule, even when
                // a role appears or goes above it.
                let previous_role = self.selected_role().map(|r| r.name.clone());
                let previous_rule = self.selected_rule().map(|r| (r.kind, r.name.clone()));
                self.roles = roles;
                self.role_errors = errors;
                self.rule_selected = match (previous_role, previous_rule) {
                    (Some(name), _) => self.roles.iter().position(|r| r.name == name),
                    (None, Some((k, n))) => self
                        .rules
                        .iter()
                        .position(|r| r.kind == k && r.name == n)
                        .map(|i| i + self.roles.len()),
                    (None, None) => None,
                }
                .unwrap_or(self.rule_selected)
                .min(self.book_len().saturating_sub(1));
            }
            Reply::Rules { rules, errors } => {
                let previous = self
                    .selected_rule()
                    .map(|r| (r.kind, r.name.clone()));
                self.rules = rules;
                self.rule_errors = errors;
                self.rule_selected = previous
                    .and_then(|(k, n)| self.rules.iter().position(|r| r.kind == k && r.name == n))
                    .map(|i| i + self.roles.len())
                    .unwrap_or(self.rule_selected)
                    .min(self.book_len().saturating_sub(1));
            }
            Reply::RuleFile { path } | Reply::RoleFile { path } => {
                if std::mem::take(&mut self.awaiting_rule_file) {
                    self.edit_request = Some(path);
                } else {
                    self.status = format!("écrit : {}", path.display());
                }
                self.request_rules();
            }
            Reply::Todos { todos } => {
                let previous = self.selected_todo().map(|t| t.id);
                self.todos = todos;
                self.todo_selected = previous
                    .and_then(|id| self.todos.iter().position(|t| t.id == id))
                    .unwrap_or(0)
                    .min(self.todos.len().saturating_sub(1));
            }
            Reply::Agents { agents } => self.adopt_live_agent(agents),
            Reply::Status { status } => {
                self.daemon_version = Some(status.version.clone());
            }
            // The pane is on screen already; what the line adds is which one,
            // for a session where a dozen of them are open.
            Reply::Pane { pane_id } => {
                self.status = format!("pane {pane_id}");
                self.refresh_open_ticket(None);
            }
            Reply::Pong | Reply::Ack | Reply::Subscribed { .. } => {}
            _ => {}
        }
    }

    pub(super) fn on_event(&mut self, e: Event) {
        // What the orchestrator does while it composes the team.
        self.trace_planning(&e);
        // An epic that moved: the list and, if it is open, the epic itself.
        if let EventKind::EpicCreated { epic_id, .. }
        | EventKind::EpicSplitReady { epic_id, .. }
        | EventKind::EpicSplitFailed { epic_id, .. }
        | EventKind::EpicAccepted { epic_id, .. }
        | EventKind::EpicFinished { epic_id } = &e.kind
        {
            self.request_epics();
            if self.epic.as_ref().is_some_and(|d| d.epic.id == *epic_id) {
                self.outbox.push(Command::GetEpic { epic_id: *epic_id });
            }
        }
        // While watching one agent, its events feed the live log.
        if self.screen == Screen::Agent && e.agent_id == self.watched_agent_id() {
            if self.log.push_event(&e) {
                self.last_activity = Some(e.ts);
            }
            if matches!(
                e.kind,
                EventKind::AgentStatusChanged { .. } | EventKind::Usage { .. }
            ) {
                self.refresh_open_ticket(e.ticket_id);
            }
            return;
        }
        if let Some(line) = describe(&e) {
            self.activity.push(line);
            if self.activity.len() > ACTIVITY_MAX {
                self.activity.drain(..self.activity.len() - ACTIVITY_MAX);
            }
        }
        // Anything that changes a board number triggers a targeted refresh.
        match &e.kind {
            EventKind::ProjectAdded { .. }
            | EventKind::ProjectForgotten { .. }
            | EventKind::ProjectMoved { .. }
            | EventKind::UnmanagedSessionSeen { .. } => {
                self.outbox.push(Command::ListProjects);
            }
            EventKind::TodoAdded { .. }
            | EventKind::TodoUpdated { .. }
            | EventKind::TodoStatusChanged { .. }
            | EventKind::TodoDeleted { .. } => {
                self.outbox.push(Command::ListTodos);
            }
            EventKind::RuleProposed { .. }
            | EventKind::RuleCreated { .. }
            | EventKind::RuleStatusChanged { .. }
            | EventKind::RuleDeleted { .. }
            | EventKind::RoleCreated { .. }
            | EventKind::RoleUpdated { .. }
            | EventKind::RoleDeleted { .. } => self.request_rules(),
            EventKind::TodoPromoted { .. } => {
                self.outbox.push(Command::ListTodos);
                self.request_tickets();
            }
            EventKind::TicketCreated { .. }
            | EventKind::TicketStatusChanged { .. }
            | EventKind::AgentStatusChanged { .. } => {
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::ProposalReady { .. } => {
                self.stop_planning();
                self.status = "proposition prête — « a » pour la relire".into();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::ProposalFailed { error } => {
                self.stop_planning();
                self.status = format!("planification échouée : {error}");
            }
            EventKind::PullRequestOpened { url, .. } => {
                self.status = format!("pull request ouverte : {url}");
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::PullRequestClosed { merged, .. } => {
                self.status = if *merged {
                    "pull request fusionnée — ticket terminé".into()
                } else {
                    "pull request fermée sans fusion".into()
                };
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            // A gate that refuses is the one thing that changes what the
            // ticket can do next, so it is said out loud rather than left in
            // the activity strip.
            EventKind::CheckFinished { run, .. } => {
                if !run.ok {
                    self.status = format!("✗ {}", run.label_fr());
                }
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::CheckStarted { command, .. } => {
                self.status = format!("vérification : {command}…");
            }
            // Two moments the user must not have to go looking for.
            EventKind::ReviewVerdict { round, verdict, .. } => {
                self.status = format!("relecture {round} : {}", verdict.label_fr());
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::MergeBlocked { reason, .. } => {
                self.status = format!(
                    "{} fusion en attente : {reason} — « f » pour réessayer",
                    crate::theme::merge_waiting().symbol
                );
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            EventKind::TicketMerged {
                branch,
                into,
                pushed_to,
                ..
            } => {
                self.status = match pushed_to {
                    Some(remote) => format!("{branch} fusionnée dans {into}, poussée sur {remote}"),
                    None => format!("{branch} fusionnée dans {into}"),
                };
                self.request_tickets();
                self.refresh_open_ticket(e.ticket_id);
            }
            // A managed agent reports every response. The cost view is polled
            // instead, so a busy agent cannot spin the loop with queries.
            EventKind::Usage { .. } => {}
            _ => {}
        }
    }
}
