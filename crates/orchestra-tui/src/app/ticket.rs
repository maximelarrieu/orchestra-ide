//! The ticket screen: its keys, planning, and what it may offer.

use super::*;

impl App {
    pub(super) fn on_ticket_char(&mut self, c: char) {
        let Some(detail) = self.ticket.as_ref() else {
            return;
        };
        let ticket_id = detail.ticket.id;
        match c {
            'p' => self.start_planning(ticket_id),
            'a' => self.open_editor(),
            'n' => self.open_new_ticket(),
            'L' => {
                self.outbox.push(Command::LaunchTicket {
                    ticket_id,
                    open_panes: false,
                });
                self.status = "lancement de l'équipe…".into();
            }
            'x' => {
                let number = detail.ticket.number;
                self.ask(
                    format!("Arrêter le ticket #{number} et ses agents ?"),
                    Command::CancelTicket { ticket_id },
                );
            }
            // Only offered on a branch the relecture cleared; the daemon
            // refuses it in every other case anyway.
            'f' if self.can_integrate() => {
                let branch = detail.ticket.branch.clone().unwrap_or_default();
                let into = detail.project.default_branch.clone();
                let question = match detail.integration_mode {
                    _ if detail.merge_blocked.is_some() => format!(
                        "Réessayer la fusion de « {branch} » dans « {into} » ? Sans agent si \
                         la branche n'a pas bougé depuis."
                    ),
                    orchestra_core::config::IntegrationMode::Pr => {
                        format!("Pousser « {branch} » et ouvrir une pull request vers « {into} » ?")
                    }
                    _ => format!("Intégrer « {branch} » dans « {into} » et terminer le ticket ?"),
                };
                self.ask(question, Command::IntegrateTicket { ticket_id });
            }
            // A ticket closed one way, to be closed another: the branch and the
            // verdict are still there, only the decision is taken back.
            'o' if detail.ticket.status.is_terminal() => {
                let number = detail.ticket.number;
                self.ask(
                    format!("Rouvrir le ticket #{number} ? Il repasse « à relire »."),
                    Command::ReopenTicket { ticket_id },
                );
            }
            't' if detail.ticket.status == orchestra_core::model::TicketStatus::Review => {
                let number = detail.ticket.number;
                self.ask(
                    format!("Marquer le ticket #{number} terminé, sans rien fusionner ?"),
                    Command::FinishTicket { ticket_id },
                );
            }
            _ => {}
        }
    }

    /// True when the open ticket can be handed to the integrator: relu, rien
    /// ne bloque, une branche à livrer, et rien déjà en vol.
    pub fn can_integrate(&self) -> bool {
        let Some(detail) = self.ticket.as_ref() else {
            return false;
        };
        detail.ticket.status == orchestra_core::model::TicketStatus::Review
            && detail.ticket.branch.is_some()
            && detail.review.as_ref().is_some_and(|r| r.is_ready())
            // A green relecture on a red build integrates nothing: the daemon
            // refuses it too, and offering the key would only produce an
            // error the user did not ask for.
            && detail.checks.as_ref().is_none_or(|c| c.passed())
            // A request is already waiting for its human: pressing the key
            // again would only reopen the same one.
            && detail.pull_request.is_none()
    }

    /// What the integration key does on this project, in the user's words.
    pub fn integration_label(&self) -> &'static str {
        if self.ticket.as_ref().is_some_and(|d| d.merge_blocked.is_some()) {
            return "réessayer la fusion";
        }
        match self.ticket.as_ref().map(|d| d.integration_mode) {
            Some(orchestra_core::config::IntegrationMode::Pr) => "ouvrir la pull request",
            _ => "intégrer et terminer",
        }
    }

    /// Ask for a team, and start following what the orchestrator does.
    ///
    /// Planning is one long silence: the run reads files for half a minute
    /// before answering. So the screen subscribes to the ticket's own events —
    /// verbose ones included — and shows them while it waits.
    pub(super) fn start_planning(&mut self, ticket_id: TicketId) {
        self.outbox.push(Command::PlanTicket { ticket_id });
        self.planning = true;
        self.planning_since = Some(orchestra_core::now());
        self.planning_trace.clear();
        self.outbox.push(Command::Subscribe {
            filter: orchestra_core::events::EventFilter::for_ticket(ticket_id),
            since_seq: None,
            backlog: 0,
        });
        self.status = "l'orchestrateur compose l'équipe…".into();
    }

    /// Back from a single agent's stream to the wider one: the planned
    /// ticket's while the orchestrator works, the board's otherwise.
    pub(super) fn subscribe_wide(&mut self) {
        let filter = match (self.planning, self.ticket.as_ref()) {
            (true, Some(detail)) => orchestra_core::events::EventFilter::for_ticket(detail.ticket.id),
            _ => orchestra_core::events::EventFilter::board(),
        };
        self.outbox.push(Command::Subscribe {
            filter,
            since_seq: None,
            backlog: 0,
        });
    }

    /// Planning is over, whatever its outcome: back to the board's stream.
    pub(super) fn stop_planning(&mut self) {
        if !self.planning {
            return;
        }
        self.planning = false;
        self.planning_since = None;
        // An agent being watched has its own subscription; leave it alone.
        if self.screen != Screen::Agent {
            self.outbox.push(Command::Subscribe {
                filter: orchestra_core::events::EventFilter::board(),
                since_seq: None,
                backlog: 0,
            });
        }
    }

    /// Fold one event into the planning trace.
    pub(super) fn trace_planning(&mut self, e: &Event) {
        if !self.planning || e.ticket_id.is_none() {
            return;
        }
        if e.ticket_id != self.ticket.as_ref().map(|d| d.ticket.id) {
            return;
        }
        if let Some(line) = plan_line(e) {
            if self.planning_trace.len() == PLAN_TRACE_MAX {
                self.planning_trace.remove(0);
            }
            self.planning_trace.push(line);
        }
    }

    /// The orchestrator run of the open ticket, most recent first.
    pub fn orchestrator_agent(&self) -> Option<&orchestra_core::protocol::AgentSummary> {
        self.ticket
            .as_ref()?
            .agents
            .iter()
            .filter(|a| a.agent.role == orchestra_core::model::ORCHESTRATOR_ROLE)
            .max_by_key(|a| a.agent.started_at)
    }
}
