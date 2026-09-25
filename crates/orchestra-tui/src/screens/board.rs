//! Board: projects on the left, their tickets on the right, activity below.
//!
//! Below 100 columns the two panes stack so the dashboard stays readable in a
//! narrow zellij pane.

use orchestra_core::pricing::{fmt_tokens, fmt_usd};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, BoardPane, Lane, Screen};
use crate::keys::{self, Hint, Scope};
use crate::theme;

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
            Constraint::Length(1),
        ])
        .split(area);

    render_tabs(app, frame, chunks[0]);
    match app.screen {
        Screen::Board => render_board(app, frame, chunks[1]),
        Screen::Cost => super::cost::render(app, frame, chunks[1]),
        Screen::Ticket => super::ticket::render(app, frame, chunks[1]),
        Screen::Agent => super::agent::render(app, frame, chunks[1]),
        Screen::NewTicket => super::new_ticket::render(app, frame, chunks[1]),
        Screen::Proposal => super::proposal::render(app, frame, chunks[1]),
    }
    render_activity(app, frame, chunks[2]);
    render_keys(app, frame, chunks[3]);
    render_status(app, frame, chunks[4]);

    if app.show_help {
        render_help(app, frame, area);
    }
    if let Some(buf) = &app.palette {
        render_palette(buf, frame, area);
    }
    if let Some(confirm) = &app.confirm {
        render_confirm(&confirm.question, frame, area);
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
        lines.push(Line::from(Span::styled(
            truncate(&format!("  {}", foot.join(" · ")), width),
            Style::default().add_modifier(Modifier::DIM),
        )));
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

/// Cut to `width` characters, with an ellipsis when something was cut.
fn truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
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
    // The keys live one line up: the status bar says the state and nothing
    // more, rather than repeating what the key bar already announces.
    let right = format!(
        "{} msg · {} tokens · {cost} (indicatif)",
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

/// The key bar, under the activity strip.
///
/// What the screen can do comes first, then a bar, then what works everywhere.
/// When room runs short it is the common keys that go: they are in the help,
/// the others are written nowhere else.
fn render_keys(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let total = area.width as usize;
    let mut hints = keys::strip_for_width(app, total);
    // Too long even at its shortest: the last common key is the way out, so
    // its room is set aside before filling — otherwise it is precisely the one
    // the trim carries off.
    let tail = match hints.last() {
        Some(last) if last.scope == Scope::Global && keys::line_width(&hints) > total => {
            hints.pop()
        }
        _ => None,
    };
    let reserved = tail
        .as_ref()
        .map(|t| {
            keys::separator(Some(Scope::Screen), Scope::Global)
                .chars()
                .count()
                + t.width()
                + 2
        })
        .unwrap_or(0);
    let budget = total.saturating_sub(reserved);

    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0usize;
    let mut previous: Option<Scope> = None;
    let mut dropped = false;

    for (i, hint) in hints.iter().enumerate() {
        let separator = keys::separator(previous, hint.scope);
        let width = separator.chars().count() + hint.width();
        // Two columns held for the « … » that says there is more.
        let reserve = if i + 1 < hints.len() { 2 } else { 0 };
        if used + width + reserve > budget {
            dropped = true;
            break;
        }
        spans.push(Span::styled(separator, theme::bracket()));
        spans.extend(hint.spans());
        used += width;
        previous = Some(hint.scope);
    }
    if dropped {
        spans.push(Span::styled(" …", theme::key_dim()));
    }
    if let Some(tail) = tail {
        spans.push(Span::styled(
            keys::separator(previous, tail.scope),
            theme::bracket(),
        ));
        spans.extend(tail.spans());
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The help shows the screen one is on, not the other five.
///
/// A list of everything teaches nothing: what one opens the help for is what
/// this screen can do. The rest sits beside it, faded, because it does not
/// change from one screen to the next.
fn render_help(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let here = keys::screen_hints(app);
    let mut here_lines = if here.is_empty() {
        vec![
            Line::from(Span::styled(
                "Cet écran n'attend rien de particulier.",
                Style::default().add_modifier(Modifier::DIM),
            )),
            Line::from(""),
        ]
    } else {
        help_section("Sur cet écran", &here)
    };
    let move_lines = help_section("Se déplacer", &from_table(keys::NAVIGATION));
    let mut everywhere = help_section("Partout", &from_table(keys::PARTOUT));
    everywhere.extend(help_commands());

    // Side by side when the width allows, stacked otherwise, and in that
    // order: if the bottom is cut, what goes missing is what can be guessed.
    let side_by_side = area.width >= 80;
    let width = if side_by_side { 78 } else { 46 }
        .min(area.width.saturating_sub(2))
        .max(20);
    let (left, right) = if side_by_side {
        let mut left = here_lines;
        left.extend(move_lines);
        (left, everywhere)
    } else {
        here_lines.extend(everywhere);
        here_lines.extend(move_lines);
        (here_lines, Vec::new())
    };
    // The last section leaves a blank line behind it; the frame should not
    // pay for it.
    let left = trimmed(left);
    let right = trimmed(right);
    let needed = left.len().max(right.len()) as u16;
    let height = (needed + 2).min(area.height.saturating_sub(2)).max(3);
    let popup = centered(area, width, height);

    let title = format!("Aide — {}", app.screen.title_fr());
    let block = pane_block(&title, true);
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    if side_by_side {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(inner);
        frame.render_widget(Paragraph::new(left), columns[0]);
        frame.render_widget(Paragraph::new(right), columns[1]);
    } else {
        frame.render_widget(Paragraph::new(left), inner);
    }
}

/// Without the trailing blank lines.
fn trimmed(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    while lines
        .last()
        .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
    {
        lines.pop();
    }
    lines
}

/// A subheading and its keys, brackets right-aligned so the actions all line
/// up on one column.
fn help_section(title: &str, hints: &[Hint]) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        title.to_string(),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    let widest = hints
        .iter()
        .map(|h| h.key.chars().count())
        .max()
        .unwrap_or(0);
    for hint in hints {
        let (key_style, label_style) = match hint.scope {
            Scope::Screen => (theme::key(), theme::key_label()),
            Scope::Global => (theme::key_dim(), theme::key_dim()),
        };
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(widest - hint.key.chars().count() + 1)),
            Span::styled("[", theme::bracket()),
            Span::styled(hint.key.clone(), key_style),
            Span::styled("] ", theme::bracket()),
            Span::styled(hint.label.clone(), label_style),
        ]));
    }
    lines.push(Line::from(""));
    lines
}

/// The palette: typed out in full, so no brackets — these are not keys.
fn help_commands() -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "Palette « : »",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    let widest = keys::PALETTE
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    for (command, what) in keys::PALETTE {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{command:widest$}"), theme::key_label()),
            Span::styled(format!("  {what}"), theme::key_dim()),
        ]));
    }
    lines.push(Line::from(""));
    lines
}

fn from_table(rows: &[(&'static str, &'static str)]) -> Vec<Hint> {
    rows.iter().map(|(k, l)| Hint::global(*k, *l)).collect()
}

/// A question that must be answered before anything irreversible happens.
fn render_confirm(question: &str, frame: &mut Frame<'_>, area: Rect) {
    let width = (question.chars().count() as u16 + 8)
        .min(area.width.saturating_sub(4))
        .max(24);
    let popup = centered(area, width, 5);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::raw(question.to_string())),
            Line::from(""),
            Line::from(
                [
                    Hint::screen("o", "confirmer").spans(),
                    vec![Span::styled(
                        "   n'importe quelle autre touche renonce",
                        theme::key_dim(),
                    )],
                ]
                .concat(),
            ),
        ])
        .block(pane_block("Confirmer", true))
        .wrap(Wrap { trim: true }),
        popup,
    );
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
        }];
        // Still on "Tous les projets": the card must say whose ticket it is.
        assert!(app.selected_project().is_none());
        let out = draw(&app, 80, 24);
        assert!(out.contains("alpha"), "{out}");
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
