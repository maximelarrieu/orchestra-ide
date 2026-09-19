//! Board: projects on the left, their tickets on the right, activity below.
//!
//! Below 100 columns the two panes stack so the dashboard stays readable in a
//! narrow zellij pane.

use orchestra_core::pricing::{fmt_tokens, fmt_usd};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::{App, BoardPane, Screen};
use crate::keymap::HELP;

/// Width below which the board stacks its panes.
const NARROW: u16 = 100;

pub fn render(app: &App, frame: &mut Frame<'_>) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(7),
            Constraint::Length(1),
        ])
        .split(area);

    render_tabs(app, frame, chunks[0]);
    match app.screen {
        Screen::Board => render_board(app, frame, chunks[1]),
        Screen::Cost => super::cost::render(app, frame, chunks[1]),
        Screen::Ticket => super::ticket::render(app, frame, chunks[1]),
        Screen::NewTicket => super::new_ticket::render(app, frame, chunks[1]),
        Screen::Proposal => super::proposal::render(app, frame, chunks[1]),
        other => render_placeholder(other, frame, chunks[1]),
    }
    render_activity(app, frame, chunks[2]);
    render_status(app, frame, chunks[3]);

    if app.show_help {
        render_help(frame, area);
    }
    if let Some(buf) = &app.palette {
        render_palette(buf, frame, area);
    }
}

fn render_tabs(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let mut spans = Vec::new();
    for (i, s) in Screen::ALL.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, s.title_fr());
        let style = if *s == app.screen {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else if s.is_implemented() {
            Style::default()
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        spans.push(Span::styled(label, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_board(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let narrow = area.width < NARROW;
    let chunks = Layout::default()
        .direction(if narrow {
            Direction::Vertical
        } else {
            Direction::Horizontal
        })
        .constraints(if narrow {
            [Constraint::Length(5), Constraint::Min(3)]
        } else {
            [Constraint::Percentage(32), Constraint::Percentage(68)]
        })
        .split(area);

    render_projects(app, frame, chunks[0]);
    render_tickets(app, frame, chunks[1]);
}

fn render_projects(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let focused = app.board_pane == BoardPane::Projects;
    let items: Vec<ListItem> = if app.projects.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "aucun projet",
            Style::default().add_modifier(Modifier::DIM),
        )]))]
    } else {
        app.projects
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let selected = i == app.project_selected;
                let mut style = Style::default();
                if selected {
                    style = style.add_modifier(Modifier::BOLD);
                    if focused {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                }
                let marker = if p.discovered { "◦" } else { "●" };
                ListItem::new(Line::from(vec![
                    Span::raw(format!("{marker} ")),
                    Span::styled(p.name.clone(), style),
                ]))
            })
            .collect()
    };
    frame.render_widget(List::new(items).block(pane_block("Projets", focused)), area);
}

fn render_tickets(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let focused = app.board_pane == BoardPane::Tickets;
    let title = match app.selected_project() {
        Some(p) => format!("Tickets — {}", p.name),
        None => "Tickets".to_string(),
    };

    if app.tickets.is_empty() {
        let hint = if app.projects.is_empty() {
            "Ajoute un projet : `:project add <chemin>`"
        } else {
            "Aucun ticket. La création arrive en phase 2."
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

    let rows: Vec<Row> = app
        .tickets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let selected = i == app.ticket_selected;
            let mut style = Style::default();
            if selected {
                style = style.add_modifier(Modifier::BOLD);
                if focused {
                    style = style.add_modifier(Modifier::REVERSED);
                }
            }
            let team = if t.agents_total == 0 {
                "-".to_string()
            } else {
                format!("{}/{}", t.agents_done, t.agents_total)
            };
            let cost = t.cost_usd.map(fmt_usd).unwrap_or_else(|| "-".into());
            Row::new(vec![
                format!("#{}", t.ticket.number),
                t.ticket.title.clone(),
                t.ticket.status.label_fr().to_string(),
                team,
                fmt_tokens(t.tokens.total()),
                cost,
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Min(16),
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Length(9),
        ],
    )
    .header(
        Row::new(vec!["#", "Titre", "Statut", "Équipe", "Tokens", "Coût"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(pane_block(&title, focused));
    frame.render_widget(table, area);
}

fn render_activity(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let visible = area.height.saturating_sub(2) as usize;
    let start = app.activity.len().saturating_sub(visible);
    let lines: Vec<Line> = if app.activity.is_empty() {
        vec![Line::from(Span::styled(
            "en attente d'activité…",
            Style::default().add_modifier(Modifier::DIM),
        ))]
    } else {
        app.activity[start..]
            .iter()
            .map(|l| Line::from(l.clone()))
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("Activité", false)),
        area,
    );
}

fn render_status(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let dot = if app.connected { "●" } else { "○" };
    let version = app
        .daemon_version
        .as_deref()
        .map(|v| format!(" v{v}"))
        .unwrap_or_default();
    let totals = &app.cost.totals;
    let cost = totals
        .cost_usd
        .map(fmt_usd)
        .unwrap_or_else(|| "≈$-".to_string());
    let left = format!("{dot} orchestra{version}  {}", app.status);
    let right = format!(
        "{} msg · {} tokens · {cost} (indicatif)  ? aide",
        totals.messages,
        fmt_tokens(totals.tokens.total())
    );
    let pad = (area.width as usize)
        .saturating_sub(left.chars().count() + right.chars().count())
        .max(1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(left),
            Span::raw(" ".repeat(pad)),
            Span::styled(right, Style::default().add_modifier(Modifier::DIM)),
        ])),
        area,
    );
}

fn render_placeholder(screen: Screen, frame: &mut Frame<'_>, area: Rect) {
    let body = match screen {
        Screen::Ticket => {
            "Détail d'un ticket : équipe, agents, coût, événements.\nArrive en phase 2."
        }
        Screen::Agent => "Flux live d'un agent, avec consigne et annulation.\nArrive en phase 3.",
        Screen::NewTicket => "Création d'un ticket : titre et brief.\nArrive en phase 2.",
        // The cost screen draws itself; this arm never renders.
        Screen::Cost => "",
        Screen::Proposal => "Relecture et édition de l'équipe proposée.\nArrive en phase 2.",
        Screen::Board => "",
    };
    frame.render_widget(
        Paragraph::new(body)
            .style(Style::default().add_modifier(Modifier::DIM))
            .block(pane_block(screen.title_fr(), false))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    let width = 46.min(area.width.saturating_sub(4)).max(20);
    let height = (HELP.len() as u16 + 2).min(area.height.saturating_sub(2));
    let popup = centered(area, width, height);
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(keys, what)| {
            Line::from(vec![
                Span::styled(
                    format!("{keys:<16}"),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(*what),
            ])
        })
        .collect();
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(pane_block("Aide", true)), popup);
}

fn render_palette(buf: &str, frame: &mut Frame<'_>, area: Rect) {
    let width = 60.min(area.width.saturating_sub(4)).max(20);
    let popup = centered(area, width, 3);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(format!(":{buf}")).block(pane_block("Commande", true)),
        popup,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

pub fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "));
    if focused {
        block.border_style(Style::default().add_modifier(Modifier::BOLD))
    } else {
        block.border_style(Style::default().add_modifier(Modifier::DIM))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ProjectRow;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use uuid::Uuid;

    fn draw(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render(app, f)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
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
