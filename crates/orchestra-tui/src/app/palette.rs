//! The `:` command line.

use super::*;

impl App {
    pub(super) fn run_palette(&mut self, line: &str) {
        let line = line.trim();
        let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        match verb {
            "t" | "ticket" => self.open_ticket_number(rest),
            "epic" | "epopee" | "épopée" => match rest.split_once(char::is_whitespace) {
                Some(("new" | "nouvelle", title)) => self.open_new_epic(title.trim()),
                None if rest == "new" || rest == "nouvelle" => self.open_new_epic(""),
                _ => self.go(Screen::Epic),
            },
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

    /// `:t 12`: open ticket #12 of what the board holds.
    fn open_ticket_number(&mut self, rest: &str) {
        let Ok(number) = rest.trim().trim_start_matches('#').parse::<i64>() else {
            self.status = "usage : :t <numéro>".into();
            return;
        };
        let found: Vec<usize> = (0..self.tickets.len())
            .filter(|i| self.tickets[*i].ticket.number == number)
            .collect();
        let Some(&index) = found.first() else {
            self.status = format!("pas de ticket #{number} sur ce tableau");
            return;
        };
        let ticket_id = self.tickets[index].ticket.id;
        self.ticket_selected = index;
        self.agent_selected = 0;
        self.agent_pinned = false;
        self.outbox.push(Command::GetTicket { ticket_id });
        self.go(Screen::Ticket);
        if found.len() > 1 {
            // Numbers repeat across projects: say which one this is.
            self.status = format!(
                "#{number} existe dans {} projets : ouvert celui de « {} »",
                found.len(),
                self.project_name(self.tickets[index].ticket.project_id).unwrap_or("?")
            );
        }
    }

    /// Tab: finish the command's name, as far as it is unambiguous.
    pub(super) fn complete_palette(&mut self) {
        let Some(buf) = self.palette.as_mut() else {
            return;
        };
        if buf.contains(char::is_whitespace) {
            return;
        }
        let mut verbs: Vec<&str> = crate::keys::PALETTE
            .iter()
            .filter_map(|(command, _)| command.trim_start_matches(':').split_whitespace().next())
            .filter(|verb| verb.starts_with(buf.as_str()))
            .collect();
        verbs.sort_unstable();
        verbs.dedup();
        match verbs.as_slice() {
            [] => {}
            [one] => *buf = format!("{one} "),
            several => {
                let first = several[0];
                let common = (0..=first.len())
                    .rev()
                    .find(|&n| first.is_char_boundary(n) && several.iter().all(|v| v.starts_with(&first[..n])))
                    .unwrap_or(0);
                *buf = first[..common].to_string();
                self.status = several.join("  ");
            }
        }
    }

    /// ↑ / ↓: walk back through the lines run before.
    pub(super) fn recall_palette(&mut self, direction: isize) {
        if self.palette_history.is_empty() {
            return;
        }
        let last = self.palette_history.len() - 1;
        let at = match (self.palette_history_at, direction < 0) {
            (None, true) => Some(last),
            (None, false) => None,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i < last => Some(i + 1),
            (Some(_), false) => None,
        };
        self.palette_history_at = at;
        self.palette = Some(at.map(|i| self.palette_history[i].clone()).unwrap_or_default());
    }
}
