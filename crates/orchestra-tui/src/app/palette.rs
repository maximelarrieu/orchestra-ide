//! The `:` command line.

use super::*;

impl App {
    pub(super) fn run_palette(&mut self, line: &str) {
        let line = line.trim();
        let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        match verb {
            "project" | "projet" => {
                let (sub, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                match sub {
                    "add" if !arg.is_empty() => {
                        self.outbox.push(Command::AddProject {
                            path: arg.into(),
                            name: None,
                        });
                        self.status = format!("ajout du projet {arg}…");
                    }
                    "forget" | "oublie" | "oublier" if !arg.is_empty() => {
                        match self.find_project(arg) {
                            Some(p) => {
                                let (id, name) = (p.id, p.name.clone());
                                self.ask(
                                    format!("Oublier le projet « {name} » ?"),
                                    Command::ForgetProject { project_id: id },
                                );
                            }
                            None => {
                                self.status = format!("aucun projet ne correspond à « {arg} »")
                            }
                        }
                    }
                    _ => {
                        self.status =
                            "usage : :project add <chemin> | :project forget <nom>".into()
                    }
                }
            }
            "todo" => {
                let (sub, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                match sub {
                    "add" if !arg.is_empty() => {
                        self.outbox.push(Command::CreateTodo {
                            title: arg.into(),
                            notes: String::new(),
                            urgent: false,
                            due_at: None,
                        });
                        self.status = "ajout du todo…".into();
                    }
                    "urgent" if !arg.is_empty() => match self.todo_at(arg) {
                        Some(t) => self.outbox.push(Command::UpdateTodo {
                            todo_id: t.id,
                            title: t.title.clone(),
                            notes: t.notes.clone(),
                            urgent: !t.urgent,
                            due_at: t.due_at,
                        }),
                        None => self.status = format!("aucun todo « {arg} »"),
                    },
                    "due" if !arg.is_empty() => {
                        let (n, date) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
                        match (self.todo_at(n), parse_due_date(date)) {
                            (Some(t), Some(due)) => self.outbox.push(Command::UpdateTodo {
                                todo_id: t.id,
                                title: t.title.clone(),
                                notes: t.notes.clone(),
                                urgent: t.urgent,
                                due_at: Some(due),
                            }),
                            (None, _) => self.status = format!("aucun todo « {n} »"),
                            (_, None) => self.status = "date invalide — AAAA-MM-JJ".into(),
                        }
                    }
                    "open" | "doing" | "done" | "drop" if !arg.is_empty() => {
                        let status = match sub {
                            "open" => TodoStatus::Open,
                            "doing" => TodoStatus::InProgress,
                            "done" => TodoStatus::Done,
                            _ => TodoStatus::Dropped,
                        };
                        match self.todo_at(arg) {
                            Some(t) => self.outbox.push(Command::SetTodoStatus {
                                todo_id: t.id,
                                status,
                            }),
                            None => self.status = format!("aucun todo « {arg} »"),
                        }
                    }
                    _ => {
                        self.status = "usage : :todo add <titre> | urgent|due|open|doing|done|drop <n>".into()
                    }
                }
            }
            "convention" | "adr" => {
                let kind = if verb == "adr" {
                    RuleKind::Adr
                } else {
                    RuleKind::Convention
                };
                let (sub, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let arg = arg.trim();
                match sub {
                    "add" if !arg.is_empty() => {
                        let project_id = self.selected_project().map(|p| p.id);
                        if kind == RuleKind::Adr && project_id.is_none() {
                            self.status = "un ADR appartient à un projet : choisis-en un sur le tableau".into();
                            return;
                        }
                        self.outbox.push(Command::CreateRule {
                            project_id,
                            rule_kind: kind,
                            title: arg.into(),
                        });
                        self.awaiting_rule_file = true;
                        self.status = format!("création de la {}…", kind.label_fr());
                    }
                    _ => self.status = format!("usage : :{verb} add <titre>"),
                }
            }
            "role" | "rôle" => {
                let (sub, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let arg = arg.trim();
                match sub {
                    "add" if !arg.is_empty() => {
                        self.outbox.push(Command::CreateRole {
                            project_id: self.selected_project().map(|p| p.id),
                            name: arg.into(),
                        });
                        self.awaiting_rule_file = true;
                        self.status = format!("création du rôle « {arg} »…");
                    }
                    _ => self.status = "usage : :role add <nom>".into(),
                }
            }
            "regles" | "règles" | "rules" | "roles" | "rôles" => self.go(Screen::Rules),
            "usage" | "cout" | "coût" => self.go(Screen::Cost),
            "refresh" => self.refresh(),
            "q" | "quit" => self.should_quit = true,
            "" => {}
            other => self.status = format!("commande inconnue : {other}"),
        }
    }

    /// The nth todo as shown on screen (1-based), what `:todo` commands take.
    pub(super) fn todo_at(&self, spec: &str) -> Option<&Todo> {
        let n: usize = spec.trim().parse().ok()?;
        n.checked_sub(1).and_then(|i| self.todos.get(i))
    }
}
