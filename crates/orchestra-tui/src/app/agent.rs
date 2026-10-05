//! The agent screen: which agent is watched, and its keys.

use super::*;

impl App {
    /// Watch an agent of the open ticket.
    ///
    /// Opening a ticket lands on the agent that is working, not on the first
    /// row: the orchestrator sorts before the team and watching it while the
    /// real agent runs looks exactly like a frozen screen.
    pub(super) fn open_agent(&mut self) {
        self.select_liveliest_agent();
        let Some(agent_id) = self.watched_agent_id() else {
            self.status = "aucun agent sur ce ticket".into();
            return;
        };
        self.log.clear();
        self.screen = Screen::Agent;
        // The backlog of this agent arrives through the event stream.
        self.outbox.push(Command::Subscribe {
            filter: orchestra_core::events::EventFilter::for_agent(agent_id),
            since_seq: None,
            backlog: 500,
        });
    }

    /// Prefer a running agent, then the most recently started one.
    pub(super) fn select_liveliest_agent(&mut self) {
        let Some(detail) = self.ticket.as_ref() else {
            return;
        };
        if self
            .watched_agent()
            .is_some_and(|a| a.agent.status.is_active())
        {
            return;
        }
        let running = detail
            .agents
            .iter()
            .position(|a| a.agent.status.is_active());
        // Nothing is working: there is no team to follow, so the row the user
        // chose himself wins. Without this, every row of a finished ticket
        // opened the last agent that ran.
        if self.agent_pinned && running.is_none() {
            return;
        }
        let fallback = || {
            detail
                .agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.agent.role != orchestra_core::model::ORCHESTRATOR_ROLE)
                .max_by_key(|(_, a)| a.agent.started_at)
                .map(|(i, _)| i)
                .or(detail.agents.len().checked_sub(1))
        };
        if let Some(index) = running.or_else(fallback) {
            self.agent_selected = index;
        }
    }

    pub(super) fn on_agent_char(&mut self, c: char) {
        let Some(agent) = self.watched_agent() else {
            return;
        };
        let active = agent.agent.status.is_active();
        let agent_id = agent.agent.id;
        match c {
            'D' => self.request_diff(),
            'o' => {
                self.outbox.push(Command::OpenPane { agent_id });
                self.status = "ouverture du pane…".into();
            }
            // Only once it has stopped: two hands on one Claude session would
            // each undo the other's turn, and the daemon refuses it anyway.
            'T' if !active => {
                self.outbox.push(Command::TakeOver { agent_id });
                self.status = "reprise en main…".into();
            }
            's' if active => {
                self.steer = Some(String::new());
                self.steer_hard = false;
            }
            'S' if active => {
                self.steer = Some(String::new());
                self.steer_hard = true;
            }
            'x' if active => {
                let role = agent.agent.role.clone();
                self.ask(
                    format!("Arrêter l'agent « {role} » ? Son travail en cours sera perdu."),
                    Command::CancelAgent { agent_id },
                );
            }
            _ => {}
        }
    }
}
