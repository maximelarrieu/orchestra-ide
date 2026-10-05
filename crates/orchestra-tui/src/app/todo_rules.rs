//! The to-do list and the rules screen: their keys and actions.

use super::*;

impl App {
    pub(super) fn on_todo_char(&mut self, c: char) {
        match c {
            'd' => self.delete_selected_todo(),
            'p' => self.promote_selected_todo(),
            _ => {}
        }
    }

    pub(super) fn on_rules_char(&mut self, c: char) {
        if self.selected_role().is_some() {
            self.on_role_char(c);
            return;
        }
        let Some(r) = self.selected_rule() else {
            self.status = "aucune règle sélectionnée".into();
            return;
        };
        let project_id = self.selected_project().map(|p| p.id);
        let (kind, name, title) = (r.kind, r.name.clone(), r.title.clone());
        match c {
            'y' => self.outbox.push(Command::SetRuleStatus {
                project_id,
                rule_kind: kind,
                name,
                status: RuleStatus::Accepted,
            }),
            'x' => self.ask(
                format!("Rejeter la {} « {title} » ?", kind.label_fr()),
                Command::SetRuleStatus {
                    project_id,
                    rule_kind: kind,
                    name,
                    status: RuleStatus::Rejected,
                },
            ),
            // Only a decision is superseded; a convention is simply rejected.
            's' if kind == RuleKind::Adr => self.outbox.push(Command::SetRuleStatus {
                project_id,
                rule_kind: kind,
                name,
                status: RuleStatus::Superseded,
            }),
            'e' => {
                if r.source.exists() {
                    self.edit_request = Some(r.source.clone());
                } else {
                    self.status =
                        "convention livrée, pas encore installée : lance « orchestra init »".into();
                }
            }
            'g' => match (project_id, r.scope) {
                (Some(project_id), orchestra_core::model::RoleScope::Project)
                    if kind == RuleKind::Convention =>
                {
                    self.ask(
                        format!("Rendre « {title} » globale, pour tous les projets ?"),
                        Command::PromoteRule { project_id, name },
                    )
                }
                _ => self.status = "seule une convention de projet peut devenir globale".into(),
            },
            'd' => self.ask(
                format!("Supprimer la {} « {title} » ?", kind.label_fr()),
                Command::DeleteRule {
                    project_id,
                    rule_kind: kind,
                    name,
                },
            ),
            _ => {}
        }
    }

    /// A role is a file: edited by hand, its git opened or closed, moved to
    /// the global catalog, deleted. Created from the palette (`:role add`).
    pub(super) fn on_role_char(&mut self, c: char) {
        let Some(r) = self.selected_role() else {
            return;
        };
        let project_id = self.selected_project().map(|p| p.id);
        let (name, scope, source) = (r.name.clone(), r.scope, r.source.clone());
        let git = r.git.unwrap_or_default();
        match c {
            'e' => self.edit_request = Some(source),
            // Opening git is outward-facing — a push leaves the machine — so it
            // is asked; closing it takes nothing away that cannot be given back.
            'p' => match git {
                GitPolicy::Confined => self.ask(
                    format!(
                        "Donner git complet (push, merge, rebase) au rôle « {name} » ? \
                         Il restera dans son worktree."
                    ),
                    Command::SetRoleGit {
                        project_id,
                        name,
                        git: GitPolicy::Full,
                    },
                ),
                GitPolicy::Full => self.outbox.push(Command::SetRoleGit {
                    project_id,
                    name,
                    git: GitPolicy::Confined,
                }),
            },
            'g' => match (project_id, scope) {
                (Some(project_id), orchestra_core::model::RoleScope::Project) => self.ask(
                    format!("Rendre le rôle « {name} » global, pour tous les projets ?"),
                    Command::PromoteRole { project_id, name },
                ),
                _ => self.status = "seul un rôle de projet peut devenir global".into(),
            },
            'd' => self.ask(
                match scope {
                    orchestra_core::model::RoleScope::Project => {
                        format!("Supprimer la version du projet du rôle « {name} » ?")
                    }
                    orchestra_core::model::RoleScope::Global => {
                        format!("Supprimer le rôle « {name} », pour tous les projets ?")
                    }
                },
                Command::DeleteRole { project_id, name },
            ),
            _ => {}
        }
    }

    pub(super) fn delete_selected_todo(&mut self) {
        let Some(t) = self.selected_todo() else {
            self.status = "aucun todo à supprimer".into();
            return;
        };
        self.ask(
            format!("Supprimer le todo « {} » ?", t.title),
            Command::DeleteTodo { todo_id: t.id },
        );
    }

    /// Seed the new-ticket form from the selected todo and remember it, so
    /// submitting promotes it instead of creating an unrelated ticket.
    pub(super) fn promote_selected_todo(&mut self) {
        if self.selected_project().is_none() {
            self.status = "choisis d'abord un projet sur le tableau".into();
            return;
        }
        let Some(t) = self.selected_todo() else {
            self.status = "aucun todo à promouvoir".into();
            return;
        };
        let (id, title, notes) = (t.id, t.title.clone(), t.notes.clone());
        self.form.seed(&title, &notes);
        self.form_field = TicketField::Title;
        self.promoting_todo = Some(id);
        self.screen = Screen::NewTicket;
    }
}
