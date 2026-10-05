//! The board's own keys: projects and new tickets.

use super::*;

impl App {
    pub(super) fn on_board_char(&mut self, c: char) {
        match c {
            'n' => self.open_new_ticket(),
            'd' if self.board_pane == BoardPane::Projects => self.forget_selected_project(),
            _ => {}
        }
    }

    /// Ask to remove the highlighted project. The daemon refuses while it
    /// still holds tickets, and that refusal comes back as an ordinary
    /// status line rather than something checked here.
    pub(super) fn forget_selected_project(&mut self) {
        let Some(p) = self.selected_project() else {
            self.status = "aucun projet à oublier".into();
            return;
        };
        self.ask(
            format!("Oublier le projet « {} » ?", p.name),
            Command::ForgetProject { project_id: p.id },
        );
    }

    /// A project by id, or by a fragment of its name or path: what
    /// `:project forget` takes, since a name is what someone types.
    pub(super) fn find_project(&self, spec: &str) -> Option<&ProjectRow> {
        let needle = spec.to_lowercase();
        self.projects.iter().find(|p| {
            p.id.to_string() == spec
                || p.name.to_lowercase().contains(&needle)
                || p.path.to_lowercase().contains(&needle)
        })
    }

    pub(super) fn open_new_ticket(&mut self) {
        if self.selected_project().is_none() {
            self.status = if self.projects.is_empty() {
                "ajoute d'abord un projet : `:project add <chemin>`".into()
            } else {
                "choisis un projet plutôt que « Tous les projets »".into()
            };
            return;
        }
        self.form.clear();
        self.form_field = TicketField::Title;
        self.promoting_todo = None;
        self.screen = Screen::NewTicket;
    }
}
