//! The team editor: opening it, its keys, accepting the team.

use super::*;

impl App {
    /// Open the team editor on the ticket's proposal, or its accepted team.
    pub(super) fn open_editor(&mut self) {
        let Some(detail) = self.ticket.as_ref() else {
            return;
        };
        let proposal = detail.ticket.proposal.clone().or_else(|| {
            detail
                .ticket
                .team
                .as_ref()
                .map(|t| orchestra_core::model::TeamProposal {
                    summary: String::new(),
                    members: t.members.clone(),
                    risks: Vec::new(),
                    estimated_size: orchestra_core::model::Size::M,
                })
        });
        match proposal {
            Some(p) => {
                self.editor.load(&p, self.roles.clone());
                self.screen = Screen::Proposal;
            }
            None => {
                self.status = "pas encore de proposition — « p » pour planifier".into();
            }
        }
    }

    pub(super) fn on_proposal_char(&mut self, c: char) {
        match c {
            'm' => {
                let aliases = self.model_aliases.clone();
                self.editor.cycle_model(&aliases);
            }
            'e' => self.editor.cycle_effort(),
            'd' => self.editor.remove_selected(),
            'a' => self.editor.add_next_role(),
            'J' => self.editor.reorder(1),
            'K' => self.editor.reorder(-1),
            'o' => self.editor.start_editing_objective(),
            'y' => self.accept_team(),
            'r' => {
                if let Some(ticket_id) = self.ticket.as_ref().map(|d| d.ticket.id) {
                    self.start_planning(ticket_id);
                    self.screen = Screen::Ticket;
                    self.status = "nouvelle proposition demandée…".into();
                }
            }
            _ => {}
        }
    }

    pub(super) fn accept_team(&mut self) {
        let Some(detail) = self.ticket.as_ref() else {
            return;
        };
        match self.editor.stages() {
            Ok(stages) => {
                self.outbox.push(Command::AcceptProposal {
                    ticket_id: detail.ticket.id,
                    team: orchestra_core::model::Team {
                        members: self.editor.members.clone(),
                        stages,
                    },
                });
                self.screen = Screen::Ticket;
                self.status = "équipe acceptée".into();
            }
            Err(e) => self.editor.error = Some(e),
        }
    }
}
