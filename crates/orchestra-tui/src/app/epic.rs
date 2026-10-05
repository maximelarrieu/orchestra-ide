//! The epics screen: the list, a split to read and accept, the tickets an
//! accepted epic became.

use orchestra_core::epic::EpicStatus;

use super::*;

impl App {
    pub(super) fn request_epics(&mut self) {
        self.outbox.push(Command::ListEpics {
            project_id: self.selected_project().map(|p| p.id),
        });
    }

    pub fn selected_epic(&self) -> Option<&orchestra_core::epic::Epic> {
        self.epics.get(self.epic_selected)
    }

    /// Splits that wait to be read: what the board's badge counts.
    pub fn splits_waiting(&self) -> usize {
        self.epics.iter().filter(|e| e.status == EpicStatus::Split).count()
    }

    /// The new-ticket form, writing an epic instead.
    pub(super) fn open_new_epic(&mut self, title: &str) {
        if self.selected_project().is_none() {
            self.status = "choisis d'abord un projet sur le tableau".into();
            return;
        }
        self.form.clear();
        self.form.seed(title, "");
        self.form_field = if title.is_empty() { TicketField::Title } else { TicketField::Brief };
        self.promoting_todo = None;
        self.creating_epic = true;
        self.screen = Screen::NewTicket;
    }

    pub(super) fn on_epic_char(&mut self, c: char) {
        let Some(detail) = self.epic.as_ref() else {
            match c {
                'n' => self.open_new_epic(""),
                'p' => {
                    if let Some(e) = self
                        .selected_epic()
                        .filter(|e| matches!(e.status, EpicStatus::Draft | EpicStatus::Split))
                    {
                        self.outbox.push(Command::PlanEpic { epic_id: e.id });
                        self.status = "l'orchestrateur découpe l'épopée…".into();
                    }
                }
                _ => {}
            }
            return;
        };
        let (epic_id, status) = (detail.epic.id, detail.epic.status);
        match (c, status) {
            ('p', EpicStatus::Draft | EpicStatus::Split) => {
                self.outbox.push(Command::PlanEpic { epic_id });
                self.status = "l'orchestrateur découpe l'épopée…".into();
            }
            ('d', EpicStatus::Split) => {
                if let Some(split) = self.split.as_mut() {
                    split.remove(self.split_selected);
                    self.split_selected = self.split_selected.min(split.tickets.len().saturating_sub(1));
                }
            }
            ('e', EpicStatus::Split) => {
                self.split_editing = self
                    .split
                    .as_ref()
                    .and_then(|s| s.tickets.get(self.split_selected))
                    .map(|t| t.brief.clone());
            }
            ('y', EpicStatus::Split) => self.accept_split(epic_id),
            _ => {}
        }
    }

    fn accept_split(&mut self, epic_id: orchestra_core::epic::EpicId) {
        let Some(split) = self.split.clone() else {
            return;
        };
        match split.waves() {
            Ok(_) => {
                self.outbox.push(Command::AcceptEpic { epic_id, proposal: split });
                self.status = "création des tickets…".into();
            }
            Err(e) => self.status = format!("découpage à corriger : {e}"),
        }
    }

    /// Keys while a split ticket's brief is rewritten: Entrée is a new line,
    /// Ctrl-S keeps, Échap gives up.
    pub(super) fn on_split_key(&mut self, action: Action) {
        let Some(buffer) = self.split_editing.as_mut() else {
            return;
        };
        match action {
            Action::Char(c) => buffer.push(c),
            Action::Backspace => {
                buffer.pop();
            }
            Action::Submit => buffer.push('\n'),
            Action::Accept => {
                let text = self.split_editing.take().unwrap_or_default();
                if let Some(t) = self
                    .split
                    .as_mut()
                    .and_then(|s| s.tickets.get_mut(self.split_selected))
                {
                    if !text.trim().is_empty() {
                        t.brief = text.trim().to_string();
                    }
                }
            }
            Action::Cancel => self.split_editing = None,
            Action::Quit => self.should_quit = true,
            _ => {}
        }
    }

    /// Entrée on the epics screen: open the epic, or the ticket under the
    /// cursor of an accepted one.
    pub(super) fn open_epic_selection(&mut self) {
        match self.epic.as_ref() {
            None => {
                if let Some(epic_id) = self.selected_epic().map(|e| e.id) {
                    self.outbox.push(Command::GetEpic { epic_id });
                }
            }
            Some(detail) => {
                if let Some(row) = detail.tickets.get(self.split_selected) {
                    let ticket_id = row.ticket_id;
                    self.agent_selected = 0;
                    self.agent_pinned = false;
                    self.outbox.push(Command::GetTicket { ticket_id });
                    self.go(Screen::Ticket);
                }
            }
        }
    }

    /// Rows the cursor moves over on the epics screen, and the cursor.
    pub(super) fn epic_cursor(&mut self) -> (usize, &mut usize) {
        let len = match (&self.epic, &self.split) {
            (None, _) => self.epics.len(),
            (Some(d), Some(s)) if d.epic.status == EpicStatus::Split => s.tickets.len(),
            (Some(d), _) => d.tickets.len(),
        };
        if self.epic.is_none() {
            (len, &mut self.epic_selected)
        } else {
            (len, &mut self.split_selected)
        }
    }

    /// A detail came back: show it, and keep an editable copy of its split.
    pub(super) fn adopt_epic(&mut self, detail: Box<orchestra_core::protocol::EpicDetail>) {
        let same = self.epic.as_ref().is_some_and(|d| d.epic.id == detail.epic.id);
        // A split being edited is the user's: a refresh must not undo it.
        let keep_edits = same && self.split.is_some() && detail.epic.status == EpicStatus::Split;
        if !keep_edits {
            self.split = (detail.epic.status == EpicStatus::Split)
                .then(|| detail.epic.proposal.clone())
                .flatten();
            if !same {
                self.split_selected = 0;
            }
        }
        if self.creating_epic && detail.epic.status == EpicStatus::Draft && detail.epic.proposal.is_none() {
            // Just written down: the split is the next step, asked for at once.
            self.creating_epic = false;
            self.outbox.push(Command::PlanEpic { epic_id: detail.epic.id });
            self.status = "l'orchestrateur découpe l'épopée…".into();
        }
        self.epic = Some(detail);
        self.screen = Screen::Epic;
    }
}
