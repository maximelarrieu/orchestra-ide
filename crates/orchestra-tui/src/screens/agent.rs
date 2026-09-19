//! Watching one agent work, and talking to it.

use orchestra_core::pricing::{fmt_tokens, fmt_usd};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::widgets::LineKind;

use super::pane_block;

pub fn render(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let Some(agent) = app.watched_agent() else {
        frame.render_widget(
            Paragraph::new(
                "Aucun agent ouvert. Depuis un ticket, choisis un agent et appuie sur Entrée.",
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
            Constraint::Length(1),
        ])
        .split(area);

    render_header(app, frame, chunks[0], &agent.agent.role);
    render_log(app, frame, chunks[1]);
    if app.steer.is_some() {
        render_steer(app, frame, chunks[2]);
    }
    render_keys(app, frame, chunks[3]);
}

fn render_header(app: &App, frame: &mut Frame<'_>, area: Rect, role: &str) {
    let agent = app.watched_agent().expect("agent ouvert");
    let log = &app.log;
    let status = agent.agent.status;

    let mut first = vec![
        Span::styled(
            format!("{role}  "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(status.label_fr().to_string()),
    ];
    if log.thinking_chars > 0 && !log.finished {
        first.push(Span::styled(
            format!("   réfléchit ({})", fmt_tokens(log.thinking_chars as u64)),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    if !log.is_following() {
        first.push(Span::styled(
            "   défilement arrêté — G pour suivre",
            Style::default().add_modifier(Modifier::DIM),
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
                let (marker, style) = match line.kind {
                    LineKind::Text => ("  ", Style::default()),
                    LineKind::ToolRunning => ("▸ ", Style::default().add_modifier(Modifier::DIM)),
                    LineKind::ToolOk => ("✓ ", Style::default().add_modifier(Modifier::DIM)),
                    LineKind::ToolFailed => ("✗ ", Style::default().add_modifier(Modifier::BOLD)),
                    LineKind::Blocked => ("⚠ ", Style::default().add_modifier(Modifier::BOLD)),
                    LineKind::Steer => ("› ", Style::default().add_modifier(Modifier::BOLD)),
                    LineKind::Notice | LineKind::Thinking => {
                        ("· ", Style::default().add_modifier(Modifier::DIM))
                    }
                };
                Line::from(vec![
                    Span::styled(
                        format!("{:>8} ", line.stamp),
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                    Span::styled(marker.to_string(), style),
                    Span::styled(line.text.clone(), style),
                ])
            })
            .collect()
    };

    let title = match app.watched_agent() {
        Some(a) if a.agent.status.is_active() => "En direct".to_string(),
        Some(a) => format!("Terminé — {}", a.agent.status.label_fr()),
        None => "Agent".to_string(),
    };
    frame.render_widget(Paragraph::new(lines).block(pane_block(&title, true)), area);
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

fn render_keys(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let active = app
        .watched_agent()
        .map(|a| a.agent.status.is_active())
        .unwrap_or(false);
    let keys = if active {
        "s consigne   S rediriger   x annuler   j/k défiler   G suivre   q retour"
    } else {
        "j/k défiler   G suivre   q retour"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            keys,
            Style::default().add_modifier(Modifier::DIM),
        ))),
        area,
    );
}
