//! Moving about: screens, lanes, rows, and the « à toi » queue.

use super::*;

impl App {
    /// Move one column over, keeping roughly the same height. Past the first
    /// column, the focus goes back to the projects.
    pub(super) fn move_lane(&mut self, delta: isize) {
        if self.screen != Screen::Board {
            return;
        }
        if self.board_pane == BoardPane::Projects {
            if delta > 0 {
                self.board_pane = BoardPane::Tickets;
            }
            return;
        }
        let lanes = self.lanes();
        let (col, row) = match self.selected_lane() {
            Some(pos) => pos,
            None => {
                // Nothing selected in a column: take the first ticket there is.
                if let Some(first) = lanes.iter().find_map(|(_, held)| held.first()) {
                    self.ticket_selected = *first;
                }
                return;
            }
        };
        // Empty columns are stepped over: there is nothing to land on.
        let mut next = col as isize + delta;
        while next >= 0 && (next as usize) < lanes.len() {
            let held = &lanes[next as usize].1;
            if let Some(i) = held.get(row.min(held.len().saturating_sub(1))) {
                self.ticket_selected = *i;
                return;
            }
            next += delta;
        }
        if next < 0 {
            self.board_pane = BoardPane::Projects;
        }
    }

    /// Tickets that wait on the user, most urgent first, then by number.
    /// Indices into `tickets`.
    pub fn attention_queue(&self) -> Vec<usize> {
        let mut queue: Vec<usize> = (0..self.tickets.len())
            .filter(|i| self.tickets[*i].attention.is_some())
            .collect();
        queue.sort_by_key(|i| {
            let t = &self.tickets[*i];
            (t.attention, t.ticket.number)
        });
        queue
    }

    /// Go to the next ticket that waits on the user, from wherever we are.
    ///
    /// Repeated, it walks the queue and comes back round, so a morning's
    /// worth of decisions is `!`, Entrée, decide, `q`, `!` again.
    pub(super) fn jump_to_attention(&mut self) {
        let queue = self.attention_queue();
        if queue.is_empty() {
            self.status = "rien ne t'attend".into();
            return;
        }
        let on_board = self.screen == Screen::Board && self.board_pane == BoardPane::Tickets;
        let next = match queue.iter().position(|i| *i == self.ticket_selected) {
            Some(pos) if on_board => (pos + 1) % queue.len(),
            _ => 0,
        };
        let index = queue[next];
        if self.screen != Screen::Board {
            self.go(Screen::Board);
        }
        self.board_pane = BoardPane::Tickets;
        self.ticket_selected = index;
        let t = &self.tickets[index];
        if let Some(a) = t.attention {
            self.status = format!(
                "à toi {}/{} : #{} {} — {}",
                next + 1,
                queue.len(),
                t.ticket.number,
                t.ticket.title,
                a.label_fr()
            );
        }
    }

    pub(super) fn go(&mut self, screen: Screen) {
        let entering_agent = screen == Screen::Agent && self.screen != Screen::Agent;
        let leaving_agent = screen != Screen::Agent && self.screen == Screen::Agent;
        if screen == Screen::Rules && self.screen != Screen::Rules {
            // The project may have changed on the board since the last read.
            self.request_rules();
        }
        self.screen = screen;
        // The daemon keeps one subscription per connection: whatever the
        // screen streams must be asked for again on the way in and out, or the
        // board goes on showing one agent and the agent screen shows nothing.
        if leaving_agent {
            self.subscribe_wide();
        }
        if entering_agent {
            match self.watched_agent_id() {
                Some(agent_id) => {
                    self.log.clear();
                    self.outbox.push(Command::Subscribe {
                        filter: orchestra_core::events::EventFilter::for_agent(agent_id),
                        since_seq: None,
                        backlog: 500,
                    });
                }
                None => {
                    // Reached from the tab strip rather than from a ticket:
                    // find the agent that is working, wherever it is.
                    self.outbox.push(Command::ListAgents { only_active: true });
                    self.status = "recherche d'un agent en cours…".into();
                }
            }
        }
        if !screen.is_implemented() {
            self.status = format!("« {} » arrive dans une phase suivante", screen.title_fr());
        } else {
            self.status = "connecté".into();
        }
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        let (len, sel) = match (self.screen, self.board_pane) {
            (Screen::Board, BoardPane::Projects) => {
                // +1 for "Tous les projets", index 0, ahead of the real list.
                (self.projects.len() + 1, &mut self.project_selected)
            }
            (Screen::Board, BoardPane::Tickets) => {
                // Up and down stay in the column; left and right change it.
                let lanes = self.lanes();
                match self.selected_lane() {
                    Some((col, row)) => {
                        let held = &lanes[col].1;
                        let next = (row as isize + delta).clamp(0, held.len() as isize - 1);
                        self.ticket_selected = held[next as usize];
                    }
                    None => {
                        if let Some(first) = lanes.iter().find_map(|(_, held)| held.first()) {
                            self.ticket_selected = *first;
                        }
                    }
                }
                return;
            }
            (Screen::Cost, _) => (self.cost.rows.len(), &mut self.cost.selected),
            (Screen::Todo, _) => (self.todos.len(), &mut self.todo_selected),
            (Screen::Rules, _) => (self.book_len(), &mut self.rule_selected),
            (Screen::Proposal, _) => {
                self.editor.move_selection(delta);
                return;
            }
            (Screen::Ticket, _) => {
                let count = self.ticket.as_ref().map(|d| d.agents.len()).unwrap_or(0);
                if count > 0 {
                    let next = (self.agent_selected as isize + delta).clamp(0, count as isize - 1);
                    self.agent_selected = next as usize;
                    self.agent_pinned = true;
                }
                return;
            }
            (Screen::Agent, _) => {
                // On a live log, down means towards the newest.
                if delta < 0 {
                    self.log.scroll_up(delta.unsigned_abs(), 20);
                } else {
                    self.log.scroll_down(delta as usize);
                }
                return;
            }
            _ => return,
        };
        if len == 0 {
            *sel = 0;
            return;
        }
        let next = (*sel as isize + delta).clamp(0, len as isize - 1);
        *sel = next as usize;
        if self.screen == Screen::Board && self.board_pane == BoardPane::Projects {
            self.request_tickets();
        }
    }

    pub(super) fn set_selection(&mut self, index: usize) {
        let (len, sel) = match (self.screen, self.board_pane) {
            (Screen::Board, BoardPane::Projects) => {
                (self.projects.len() + 1, &mut self.project_selected)
            }
            // The ends of a column, not of the whole board.
            (Screen::Board, BoardPane::Tickets) => {
                let lanes = self.lanes();
                if let Some((col, _)) = self.selected_lane() {
                    let held = &lanes[col].1;
                    let row = index.min(held.len().saturating_sub(1));
                    self.ticket_selected = held[row];
                }
                return;
            }
            (Screen::Cost, _) => (self.cost.rows.len(), &mut self.cost.selected),
            (Screen::Todo, _) => (self.todos.len(), &mut self.todo_selected),
            (Screen::Rules, _) => (self.book_len(), &mut self.rule_selected),
            _ => return,
        };
        *sel = index.min(len.saturating_sub(1));
        if self.screen == Screen::Board && self.board_pane == BoardPane::Projects {
            self.request_tickets();
        }
    }

    pub(super) fn open_selection(&mut self) {
        match (self.screen, self.board_pane) {
            (Screen::Board, BoardPane::Projects) => {
                self.board_pane = BoardPane::Tickets;
                self.request_tickets();
            }
            (Screen::Board, BoardPane::Tickets) => {
                if let Some(ticket_id) = self.selected_ticket_id() {
                    self.agent_selected = 0;
                    self.agent_pinned = false;
                    self.outbox.push(Command::GetTicket { ticket_id });
                    self.go(Screen::Ticket);
                }
            }
            (Screen::Proposal, _) => self.editor.start_editing_objective(),
            (Screen::Ticket, _) => self.open_agent(),
            _ => {}
        }
    }
}
