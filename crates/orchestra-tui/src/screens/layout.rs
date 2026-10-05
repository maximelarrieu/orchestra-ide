//! The frame every screen sits in: tabs, the screen itself, the activity
//! strip, the key bar and the status line, plus the overlays (help, palette,
//! confirmation).

use orchestra_core::pricing::{fmt_tokens, fmt_usd};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Screen};
use crate::keys::{self, Hint, Scope};
use crate::theme;

use super::board::render_board;

/// Height below which the activity strip shrinks to two lines.
const SHORT: u16 = 30;

/// Rows given to the activity strip, borders included.
pub fn activity_height(app: &App, height: u16) -> u16 {
    if app.activity_hidden {
        0
    } else if height < SHORT {
        // On 24 rows the full strip left the screen fourteen.
        4
    } else {
        7
    }
}

pub fn render(app: &App, frame: &mut Frame<'_>) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(activity_height(app, area.height)),
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
        Screen::Todo => super::todo::render(app, frame, chunks[1]),
        Screen::Rules => super::rules::render(app, frame, chunks[1]),
    }
    if chunks[2].height > 0 {
        render_activity(app, frame, chunks[2]);
    }
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
    // What waits on the user goes first: it is the reason to look at all.
    let waiting = app.attention_queue();
    if let Some(first) = waiting.first().and_then(|i| app.tickets[*i].attention) {
        let badge = theme::attention(first);
        spans.push(Span::styled(
            // The key rides along: on a narrow bar it is the first to go.
            format!("{} {} à toi [!] ", badge.symbol, waiting.len()),
            badge.style(),
        ));
    }
    let badges: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    // Every tab named takes about ninety columns; past that, only the open
    // one keeps its name and the others keep their number.
    let named: usize = Screen::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| format!(" {} {} ", i + 1, s.title_fr()).chars().count())
        .sum();
    let compact = named + badges + 30 > area.width as usize;
    for (i, s) in Screen::ALL.iter().enumerate() {
        let label = if compact && *s != app.screen {
            format!(" {} ", i + 1)
        } else {
            format!(" {} {} ", i + 1, s.title_fr())
        };
        let style = if *s == app.screen {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else if s.is_implemented() {
            Style::default()
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        spans.push(Span::styled(label, style));
    }
    // What the morning should look at first, visible from every screen —
    // opening Orchestra is the notification, not a separate service.
    let digest = app.todo_digest();
    if !digest.is_empty() {
        let badge = theme::urgent();
        spans.push(Span::raw("  "));
        spans.push(Span::styled(badge.symbol, badge.style()));
        spans.push(Span::styled(format!(" {}", digest.summary_fr()), badge.style()));
    }
    // A proposal is a decision waiting on the user: nothing applies it until
    // then, so it must not wait unseen.
    let pending = app.pending_rules();
    if pending > 0 {
        let badge = theme::rule(orchestra_core::conventions::RuleStatus::Proposed);
        spans.push(Span::raw("  "));
        spans.push(Span::styled(badge.symbol, badge.style()));
        spans.push(Span::styled(
            format!(" {pending} règle(s) à valider"),
            badge.style(),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Cut to `width` characters, with an ellipsis when something was cut.
pub fn truncate(text: &str, width: usize) -> String {
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
