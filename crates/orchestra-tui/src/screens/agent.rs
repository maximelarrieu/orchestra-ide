//! Watching one agent work, and talking to it.

use orchestra_core::pricing::{fmt_tokens, fmt_usd};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::theme;
use crate::widgets::LineKind;

use super::pane_block;

pub fn render(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let Some(agent) = app.watched_agent() else {
        frame.render_widget(
            Paragraph::new(
                "Aucun agent ne tourne en ce moment.\n\nDepuis un ticket, « L » lance l'équipe, puis Entrée sur un agent suit son travail en direct.",
            )
            .style(Style::default().add_modifier(Modifier::DIM))
            .block(pane_block("Agent", true))
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    };

    let input_height = if app.steer.is_some() { 3 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(input_height),
        ])
        .split(area);

    let label = app
        .ticket
        .as_ref()
        .map(|d| crate::app::agent_label(&d.agents, app.agent_selected))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| agent.agent.role.clone());
    render_header(app, frame, chunks[0], &label);
    render_log(app, frame, chunks[1]);
    if app.steer.is_some() {
        render_steer(app, frame, chunks[2]);
    }
}

fn render_header(app: &App, frame: &mut Frame<'_>, area: Rect, role: &str) {
    let agent = app.watched_agent().expect("agent ouvert");
    let log = &app.log;
    let status = agent.agent.status;
    let badge = theme::agent(status);
    let now = orchestra_core::now();

    let mut first = vec![Span::styled(
        format!("{role}  "),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    // A turning spinner says "alive" during a long silence; a still symbol
    // says the run is over.
    if status.is_active() {
        first.push(Span::styled(
            format!("{} ", theme::spinner(app.ticks)),
            badge.style(),
        ));
    }
    first.push(Span::styled(badge.label(status.label_fr()), badge.style()));
    if let Some(reason) = &agent.agent.exit_reason {
        first.push(Span::styled(
            format!(" — {}", reason.label_fr()),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    if let Some(started) = agent.agent.started_at {
        let end = agent.agent.ended_at.unwrap_or(now);
        first.push(Span::styled(
            format!("   {}", theme::elapsed(started, end)),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    if log.thinking_chars > 0 && !log.finished {
        first.push(Span::styled(
            format!("   réfléchit ({})", fmt_tokens(log.thinking_chars as u64)),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    // Long silences are normal: saying how long it has been is the difference
    // between waiting calmly and wondering whether it is stuck.
    if status.is_active() {
        if let Some(last) = app.last_activity {
            let quiet = (now - last).whole_seconds();
            if quiet >= 10 {
                first.push(Span::styled(
                    format!("   silencieux depuis {}", theme::elapsed(last, now)),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
        }
    }
    if !log.is_following() {
        first.push(Span::styled(
            "   défilement arrêté — G pour suivre",
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }

    let tokens = if log.tokens.total() > 0 {
        log.tokens
    } else {
        agent.tokens
    };
    let cache = if tokens.total_input() > 0 {
        format!(" · cache {:.0}%", tokens.cache_hit_ratio() * 100.0)
    } else {
        String::new()
    };
    let second = format!(
        "{} tours · entrée {} · sortie {}{cache} · {}{}",
        log.turns.max(agent.turns),
        fmt_tokens(tokens.total_input()),
        fmt_tokens(tokens.output),
        agent
            .cost_usd
            .map(fmt_usd)
            .unwrap_or_else(|| "≈$0".to_string()),
        agent
            .agent
            .model
            .is_empty()
            .then(String::new)
            .unwrap_or_else(|| format!(" · {}", agent.agent.model)),
    );

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(first),
            Line::from(Span::styled(
                second,
                Style::default().add_modifier(Modifier::DIM),
            )),
        ]),
        area,
    );
}

fn render_log(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let height = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = if app.log.is_empty() {
        vec![Line::from(Span::styled(
            "en attente de l'agent…",
            Style::default().add_modifier(Modifier::DIM),
        ))]
    } else {
        app.log
            .window(height)
            .into_iter()
            .map(|line| {
                let (marker, marker_style, text_style) = line_styles(line.kind);
                Line::from(vec![
                    Span::styled(
                        format!("{:>8} ", line.stamp),
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                    Span::styled(marker.to_string(), marker_style),
                    Span::styled(line.text.clone(), text_style),
                ])
            })
            .collect()
    };

    let title = match app.watched_agent() {
        Some(a) if a.agent.status.is_active() => {
            format!("En direct — {} ligne(s)", app.log.len())
        }
        Some(a) => format!("{} — {} ligne(s)", a.agent.status.label_fr(), app.log.len()),
        None => "Agent".to_string(),
    };
    frame.render_widget(Paragraph::new(lines).block(pane_block(&title, true)), area);
}

/// How one log line is drawn; the palette lives in `theme`, with the rest.
pub fn line_styles(kind: LineKind) -> (&'static str, Style, Style) {
    crate::theme::log_line(kind)
}

fn render_steer(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let buffer = app.steer.as_deref().unwrap_or("");
    let title = if app.steer_hard {
        "Rediriger — interrompt le tour en cours (Ctrl-S envoyer, Échap annuler)"
    } else {
        "Consigne — traitée après le tour en cours (Ctrl-S envoyer, Échap annuler)"
    };
    frame.render_widget(
        Paragraph::new(format!("{buffer}▏")).block(pane_block(title, true)),
        area,
    );
}
