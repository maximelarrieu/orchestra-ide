//! The board: projects on the left, their tickets in columns on the right.
//!
//! Below 100 columns the two panes stack so the dashboard stays readable in a
//! narrow zellij pane.

use orchestra_core::pricing::fmt_usd;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, BoardPane, Lane};
use crate::theme;

use super::layout::{pane_block, truncate};

/// Width below which the board stacks its panes.
const NARROW: u16 = 100;

pub(super) fn render_board(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let narrow = area.width < NARROW;
    let chunks = Layout::default()
        .direction(if narrow {
            Direction::Vertical
        } else {
            Direction::Horizontal
        })
        // The projects pane is a chooser, not a view: it needs a name's width.
        // Everything else goes to the board, which is where the work is read.
        .constraints(if narrow {
            [Constraint::Length(5), Constraint::Min(3)]
        } else {
            [Constraint::Length(22), Constraint::Min(40)]
        })
        .split(area);

    render_projects(app, frame, chunks[0]);
    render_tickets(app, frame, chunks[1]);
}

fn render_projects(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let focused = app.board_pane == BoardPane::Projects;
    let mut items: Vec<ListItem> = Vec::with_capacity(app.projects.len() + 2);
    // "Tous les projets" sits at index 0, ahead of the real list, so a
    // global view is one `j` shorter than any single project rather than a
    // separate mode to learn.
    let mut all_style = Style::default();
    if app.project_selected == 0 {
        all_style = all_style.add_modifier(Modifier::BOLD);
        if focused {
            all_style = all_style.add_modifier(Modifier::REVERSED);
        }
    }
    items.push(ListItem::new(Line::from(vec![
        Span::raw("◆ "),
        Span::styled("Tous les projets", all_style),
    ])));
    if app.projects.is_empty() {
        items.push(ListItem::new(Line::from(vec![Span::styled(
            "aucun projet",
            Style::default().add_modifier(Modifier::DIM),
        )])));
    } else {
        items.extend(app.projects.iter().enumerate().map(|(i, p)| {
            let selected = i + 1 == app.project_selected;
            let mut style = Style::default();
            if selected {
                style = style.add_modifier(Modifier::BOLD);
                if focused {
                    style = style.add_modifier(Modifier::REVERSED);
                }
            }
            // A project Orchestra only discovered is drawn faintly: it is
            // there for its costs, not to work in.
            let marker = if p.discovered { "◦" } else { "●" };
            let marker_style = if p.discovered {
                Style::default().add_modifier(Modifier::DIM)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{marker} "), marker_style),
                Span::styled(p.name.clone(), style),
            ]))
        }));
    }
    frame.render_widget(List::new(items).block(pane_block("Projets", focused)), area);
}

fn render_tickets(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let focused = app.board_pane == BoardPane::Tickets;

    if app.tickets.is_empty() {
        let title = match app.selected_project() {
            Some(p) => format!("Tickets — {}", p.name),
            None if app.projects.is_empty() => "Tickets".to_string(),
            None => "Tickets — tous les projets".to_string(),
        };
        let hint = if app.projects.is_empty() {
            "Ajoute un projet : `:project add <chemin>`"
        } else {
            "Aucun ticket. « n » en crée un."
        };
        frame.render_widget(
            Paragraph::new(hint)
                .style(Style::default().add_modifier(Modifier::DIM))
                .block(pane_block(&title, focused))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let lanes = app.lanes();
    let share = Constraint::Ratio(1, lanes.len().max(1) as u32);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![share; lanes.len()])
        .split(area);

    for ((lane, held), rect) in lanes.iter().zip(columns.iter()) {
        render_lane(app, frame, *rect, *lane, held, focused);
    }
}

/// One column of the board, and the cards in it.
fn render_lane(
    app: &App,
    frame: &mut Frame<'_>,
    area: Rect,
    lane: Lane,
    held: &[usize],
    focused: bool,
) {
    let title = if held.is_empty() {
        lane.title_fr().to_string()
    } else {
        format!("{} ({})", lane.title_fr(), held.len())
    };
    // The column holding the selection is the one that carries the focus mark.
    let holds_selection = held.contains(&app.ticket_selected);
    let block = pane_block(&title, focused && holds_selection);

    if held.is_empty() {
        frame.render_widget(block, area);
        return;
    }

    // Three lines a card, and only so many fit: the selected one is always
    // among them, so the column scrolls with the cursor rather than hiding it.
    let inner_height = area.height.saturating_sub(2) as usize;
    let per_card = 3;
    let visible = (inner_height / per_card).max(1);
    let position = held
        .iter()
        .position(|i| *i == app.ticket_selected)
        .unwrap_or(0);
    let start = position.saturating_sub(visible.saturating_sub(1));
    let width = area.width.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();
    for index in held.iter().skip(start).take(visible) {
        let Some(t) = app.tickets.get(*index) else {
            continue;
        };
        let selected = *index == app.ticket_selected;
        let mut title_style = Style::default();
        if selected {
            title_style = title_style.add_modifier(Modifier::BOLD);
            if focused {
                title_style = title_style.add_modifier(Modifier::REVERSED);
            }
        }
        let head = format!("#{} {}", t.ticket.number, t.ticket.title);
        lines.push(Line::from(Span::styled(
            truncate(&head, width),
            title_style,
        )));

        // Second line: what the card is worth and whether anyone is on it.
        let mut foot = Vec::new();
        // Pooled across every project, a bare "#N" no longer says which one
        // — ticket numbers only avoid collisions within a single project.
        if app.selected_project().is_none() {
            if let Some(name) = app.project_name(t.ticket.project_id) {
                foot.push(name.to_string());
            }
        }
        if t.agents_active > 0 {
            foot.push(format!(
                "{} {} au travail",
                theme::spinner(app.ticks),
                t.agents_active
            ));
        } else if t.agents_total > 0 {
            foot.push(format!("{}/{} agents", t.agents_done, t.agents_total));
        }
        if let Some(cost) = t.cost_usd {
            foot.push(fmt_usd(cost));
        }
        // A ready branch refused at the fusion waits on the user, not on an
        // agent: that goes first on the line, and it is not dimmed.
        let mut spans = vec![Span::raw("  ")];
        let mut used = 2;
        // A ticket of an epic still waiting says for which one: it is why
        // nothing happens to it.
        let waiting = t
            .epic
            .as_ref()
            .filter(|l| !l.waiting_on.is_empty())
            .map(|l| {
                let numbers: Vec<String> = l.waiting_on.iter().map(|n| format!("#{n}")).collect();
                format!("attend {}", numbers.join(", "))
            });
        let mark: Option<(theme::Badge, String)> = match (t.attention, &t.merge_blocked, waiting) {
            (Some(a), _, _) => Some((theme::attention(a), a.label_fr().to_string())),
            // A daemon from before the queue still says this much.
            (None, Some(_), _) => Some((theme::merge_waiting(), "fusion en attente".into())),
            (None, None, Some(w)) => Some((theme::epic_waiting(), w)),
            (None, None, None) => None,
        };
        if let Some(epic) = &t.epic {
            foot.push(epic.epic_title.clone());
        }
        if let Some((badge, label)) = mark {
            let mark = badge.label(&label);
            used += mark.chars().count() + 3;
            spans.push(Span::styled(truncate(&mark, width.saturating_sub(2)), badge.style()));
            if !foot.is_empty() {
                spans.push(Span::raw(" · "));
            }
        }
        spans.push(Span::styled(
            truncate(&foot.join(" · "), width.saturating_sub(used)),
            Style::default().add_modifier(Modifier::DIM),
        ));
        lines.push(Line::from(spans));
        lines.push(Line::from(""));
    }
    if start + visible < held.len() {
        lines.pop();
        lines.push(Line::from(Span::styled(
            format!("  + {} de plus", held.len() - start - visible),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::layout::render;
    use crate::app::ProjectRow;
    use uuid::Uuid;

    fn draw(app: &App, w: u16, h: u16) -> String {
        crate::screens::text_of(w, h, |f| render(app, f))
    }

    fn app_with_a_project() -> App {
        let mut app = App::new();
        app.connected = true;
        app.projects = vec![ProjectRow {
            id: Uuid::new_v4(),
            name: "orchestra".into(),
            path: "/tmp/orchestra".into(),
            discovered: false,
        }];
        app
    }

    #[test]
    fn renders_at_eighty_by_twentyfour() {
        let out = draw(&app_with_a_project(), 80, 24);
        assert!(out.contains("Projets"));
        assert!(out.contains("orchestra"));
        assert!(out.contains("Tableau"));
    }

    #[test]
    fn renders_in_a_narrow_pane() {
        // 60 columns is a realistic zellij side pane; it must not panic and must
        // still show both panes, stacked.
        let out = draw(&app_with_a_project(), 60, 40);
        assert!(out.contains("Projets"));
        assert!(out.contains("Tickets"));
    }

    #[test]
    fn renders_when_empty() {
        let out = draw(&App::new(), 80, 24);
        assert!(out.contains("aucun projet"));
    }

    #[test]
    fn tous_les_projets_leads_the_selector_and_is_selected_by_default() {
        let app = app_with_a_project();
        assert!(
            app.selected_project().is_none(),
            "démarre sur « Tous les projets »"
        );
        let out = draw(&app, 80, 24);
        assert!(out.contains("Tous les projets"));
        // It comes before the real project, not after.
        assert!(out.find("Tous les projets").unwrap() < out.find("orchestra").unwrap());
    }

    #[test]
    fn a_pooled_view_names_each_ticket_s_project() {
        let mut app = App::new();
        app.connected = true;
        app.board_pane = BoardPane::Tickets;
        let project_id = Uuid::new_v4();
        app.projects = vec![ProjectRow {
            id: project_id,
            name: "alpha".into(),
            path: "/tmp/alpha".into(),
            discovered: false,
        }];
        let ticket = orchestra_core::model::Ticket {
            id: Uuid::new_v4(),
            project_id,
            number: 1,
            title: "un ticket".into(),
            brief: "b".into(),
            status: orchestra_core::model::TicketStatus::Draft,
            branch: None,
            worktree_path: None,
            proposal: None,
            team: None,
            created_at: orchestra_core::now(),
            updated_at: orchestra_core::now(),
        };
        app.tickets = vec![orchestra_core::protocol::TicketSummary {
            ticket,
            agents_total: 0,
            agents_active: 0,
            agents_done: 0,
            tokens: orchestra_core::model::Tokens::default(),
            cost_usd: None,
            pull_request: None,
            merge_blocked: None,
            attention: None,
            epic: None,
        }];
        // Still on "Tous les projets": the card must say whose ticket it is.
        assert!(app.selected_project().is_none());
        let out = draw(&app, 80, 24);
        assert!(out.contains("alpha"), "{out}");
    }

    #[test]
    fn a_refused_merge_is_marked_on_its_card() {
        let mut app = App::new();
        app.connected = true;
        app.board_pane = BoardPane::Tickets;
        let project_id = Uuid::new_v4();
        app.projects = vec![ProjectRow {
            id: project_id,
            name: "alpha".into(),
            path: "/tmp/alpha".into(),
            discovered: false,
        }];
        let ticket = orchestra_core::model::Ticket {
            id: Uuid::new_v4(),
            project_id,
            number: 2,
            title: "cockpit".into(),
            brief: "b".into(),
            status: orchestra_core::model::TicketStatus::Review,
            branch: Some("orch/2-cockpit".into()),
            worktree_path: None,
            proposal: None,
            team: None,
            created_at: orchestra_core::now(),
            updated_at: orchestra_core::now(),
        };
        app.tickets = vec![orchestra_core::protocol::TicketSummary {
            ticket,
            agents_total: 3,
            agents_active: 0,
            agents_done: 3,
            tokens: orchestra_core::model::Tokens::default(),
            cost_usd: None,
            pull_request: None,
            merge_blocked: Some("le dépôt principal a 1 fichier(s) modifié(s)".into()),
            attention: None,
            epic: None,
        }];
        let out = draw(&app, 120, 30);
        assert!(out.contains("⏸ fusion en attente"), "{out}");
        assert_eq!(Lane::of(&app.tickets[0]), Lane::Review);
    }

    #[test]
    fn help_overlay_draws_over_the_board() {
        let mut app = app_with_a_project();
        app.show_help = true;
        let out = draw(&app, 80, 24);
        assert!(out.contains("Aide"));
        assert!(out.contains("palette"));
    }

    #[test]
    fn palette_shows_what_is_typed() {
        let mut app = app_with_a_project();
        app.palette = Some("project add /tmp".into());
        let out = draw(&app, 80, 24);
        assert!(out.contains(":project add /tmp"));
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        // A pane can be resized to almost nothing; rendering must survive it.
        for (w, h) in [(20u16, 6u16), (10, 4), (40, 10)] {
            let _ = draw(&app_with_a_project(), w, h);
        }
    }
}
