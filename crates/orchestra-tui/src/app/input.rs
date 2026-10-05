//! Keys: the cascade from overlays to the screen, and the text fields.

use super::*;

impl App {
    pub(super) fn on_key(&mut self, action: Action) {
        // A pending question takes every key until it is answered.
        if self.confirm.is_some() {
            self.on_confirm_key(action);
            return;
        }
        // The palette swallows keys while it is open.
        if let Some(buf) = self.palette.as_mut() {
            match action {
                Action::Char(c) => buf.push(c),
                Action::Backspace => {
                    buf.pop();
                }
                Action::Cancel => self.palette = None,
                Action::Submit => {
                    let line = self.palette.take().unwrap_or_default();
                    self.run_palette(&line);
                }
                Action::Quit => self.should_quit = true,
                _ => {}
            }
            return;
        }
        if self.show_help {
            // Any key closes the help overlay, except quitting outright.
            match action {
                Action::Quit => self.should_quit = true,
                _ => self.show_help = false,
            }
            return;
        }

        if self.screen == Screen::NewTicket {
            self.on_form_key(action);
            return;
        }
        if self.screen == Screen::Proposal && self.editor.is_editing() {
            self.on_objective_key(action);
            return;
        }
        if self.steer.is_some() {
            self.on_steer_key(action);
            return;
        }

        match action {
            Action::Quit => self.should_quit = true,
            Action::Back => self.on_back(),
            Action::Help => self.show_help = true,
            Action::CommandPalette => self.palette = Some(String::new()),
            Action::Refresh => self.refresh(),
            Action::Screen(n) => {
                if let Some(s) = Screen::from_number(n) {
                    self.go(s);
                }
            }
            Action::NextScreen => self.go(self.screen.next()),
            Action::PrevScreen => self.go(self.screen.prev()),
            Action::Left => self.move_lane(-1),
            Action::Right => self.move_lane(1),
            Action::PageUp => self.page(-1),
            Action::PageDown => self.page(1),
            Action::Up => self.move_selection(-1),
            Action::Down => self.move_selection(1),
            Action::Top => self.set_selection(0),
            Action::Bottom => {
                if self.screen == Screen::Agent {
                    self.log.follow();
                } else {
                    self.set_selection(usize::MAX)
                }
            }
            Action::Select => self.open_selection(),
            Action::Char(c) => self.on_char(c),
            _ => {}
        }
    }

    /// Escape and `q` step back one screen rather than quitting outright, so a
    /// half-written ticket is not lost to a reflex.
    pub(super) fn on_back(&mut self) {
        match self.screen {
            Screen::Board => self.should_quit = true,
            Screen::Ticket => {
                self.ticket = None;
                self.agent_selected = 0;
                self.agent_pinned = false;
                self.screen = Screen::Board;
            }
            Screen::Proposal | Screen::Agent => {
                self.screen = Screen::Ticket;
                // Back to the whole ticket's events.
                self.outbox.push(Command::Subscribe {
                    filter: orchestra_core::events::EventFilter::board(),
                    since_seq: None,
                    backlog: 50,
                });
            }
            _ => self.screen = Screen::Board,
        }
    }

    pub(super) fn on_form_key(&mut self, action: Action) {
        match action {
            Action::Char(c) => self.form.type_char(self.form_field, c),
            Action::Backspace => self.form.backspace(self.form_field),
            Action::NextField => self.form_field = self.form_field.next(),
            Action::Submit => self.form.newline(self.form_field),
            Action::Accept => self.submit_form(),
            Action::Cancel => {
                self.form.clear();
                self.promoting_todo = None;
                self.screen = Screen::Board;
            }
            Action::Quit => self.should_quit = true,
            _ => {}
        }
    }

    pub(super) fn submit_form(&mut self) {
        let Some(project) = self.selected_project().map(|p| p.id) else {
            self.form.error = Some("choisis d'abord un projet sur le tableau".into());
            return;
        };
        match self.form.validated() {
            Ok((title, brief)) => {
                match self.promoting_todo.take() {
                    Some(todo_id) => {
                        self.outbox.push(Command::PromoteTodo {
                            todo_id,
                            project_id: project,
                            title,
                            brief,
                        });
                        self.status = "promotion en ticket…".into();
                    }
                    None => {
                        self.outbox.push(Command::CreateTicket {
                            project_id: project,
                            title,
                            brief,
                        });
                        self.status = "ticket créé".into();
                    }
                }
                self.form.clear();
                self.screen = Screen::Board;
            }
            Err(e) => self.form.error = Some(e),
        }
    }

    /// `o`, `y` or Enter confirms; anything else declines.
    pub(super) fn on_confirm_key(&mut self, action: Action) {
        let accepted = matches!(
            action,
            // `y` or Entrée, nothing else: `o` used to confirm too, and is
            // « ouvrir » everywhere else.
            Action::Char('y') | Action::Select
        );
        let confirm = self.confirm.take();
        match (accepted, confirm) {
            (true, Some(c)) => {
                self.outbox.push(c.command);
                self.status = "c'est parti".into();
            }
            _ => self.status = "annulation abandonnée".into(),
        }
    }

    /// Queue a destructive action behind a question.
    pub(super) fn ask(&mut self, question: impl Into<String>, command: Command) {
        self.confirm = Some(Confirm {
            question: question.into(),
            command,
        });
    }

    pub(super) fn on_steer_key(&mut self, action: Action) {
        match action {
            Action::Char(c) => {
                if let Some(buf) = self.steer.as_mut() {
                    buf.push(c);
                }
            }
            Action::Backspace => {
                if let Some(buf) = self.steer.as_mut() {
                    buf.pop();
                }
            }
            Action::Submit | Action::Accept => self.send_steer(),
            Action::Cancel => self.steer = None,
            Action::Quit => self.should_quit = true,
            _ => {}
        }
    }

    pub(super) fn send_steer(&mut self) {
        let text = self.steer.take().unwrap_or_default().trim().to_string();
        let Some(agent_id) = self.watched_agent_id() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        self.outbox.push(Command::SteerAgent {
            agent_id,
            text,
            hard: self.steer_hard,
        });
        self.status = if self.steer_hard {
            "redirection envoyée".into()
        } else {
            "consigne envoyée".into()
        };
    }

    pub(super) fn on_objective_key(&mut self, action: Action) {
        match action {
            Action::Char(c) => self.editor.type_char(c),
            Action::Backspace => self.editor.backspace(),
            Action::Submit | Action::Accept => self.editor.finish_editing(true),
            Action::Cancel => self.editor.finish_editing(false),
            Action::Quit => self.should_quit = true,
            _ => {}
        }
    }

    pub(super) fn on_char(&mut self, c: char) {
        // Two keys that work from every screen. Neither letter is taken by
        // a screen of its own, which a test keeps true.
        match c {
            '!' => return self.jump_to_attention(),
            'A' => {
                self.activity_hidden = !self.activity_hidden;
                return;
            }
            _ => {}
        }
        match self.screen {
            Screen::Cost => self.on_cost_char(c),
            Screen::Board => self.on_board_char(c),
            Screen::Ticket => self.on_ticket_char(c),
            Screen::Agent => self.on_agent_char(c),
            Screen::Proposal => self.on_proposal_char(c),
            Screen::Todo => self.on_todo_char(c),
            Screen::Rules => self.on_rules_char(c),
            _ => {}
        }
    }
}
