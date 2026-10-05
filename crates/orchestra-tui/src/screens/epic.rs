//! Epics: the list, a split to read before anything exists, and the
//! tickets an accepted epic became, in order.

use orchestra_core::epic::EpicStatus;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::theme;

use super::{pane_block, truncate};

/// A status's symbol, so the list reads without colour.
fn symbol(status: EpicStatus) -> &'static str {
    match status {
        EpicStatus::Draft => "○",
        EpicStatus::Split => "▶",
        EpicStatus::Active => "●",
        EpicStatus::Done => "✓",
        EpicStatus::Cancelled => "⊘",
    }
}

pub fn render(app: &App, frame: &mut Frame<'_>, area: Rect) {
    match app.epic.as_ref() {
        None => render_list(app, frame, area),
        Some(detail) => match detail.epic.status {
            EpicStatus::Split => render_split(app, frame, area),
            _ => render_epic(app, frame, area),
        },
    }
    if let Some(buffer) = &app.split_editing {
        let width = area.width.saturating_sub(8).clamp(20, 90);
        let height = area.height.saturating_sub(2).clamp(3, 12);
        let popup = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(format!("{buffer}▏"))
                .block(pane_block("Brief — Ctrl-S garder, Échap annuler", true))
                .wrap(Wrap { trim: false }),
            popup,
        );
    }
}

fn render_list(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let width = area.width.saturating_sub(4) as usize;
    let lines: Vec<Line> = if app.epics.is_empty() {
        vec![Line::from(Span::styled(
            "Aucune épopée. « n » en écrit une, à découper en tickets.",
            Style::default().add_modifier(Modifier::DIM),
        ))]
    } else {
        app.epics
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let mut style = Style::default();
                if i == app.epic_selected {
                    style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
                }
                let text = format!("{} {} — {}", symbol(e.status), e.title, e.status.label_fr());
                Line::from(Span::styled(truncate(&text, width), style))
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines).block(pane_block("Épopées", true)), area);
}

fn render_split(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let (Some(detail), Some(split)) = (app.epic.as_ref(), app.split.as_ref()) else {
        return;
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(4), Constraint::Length(8)])
        .split(area);

    let order = match split.waves() {
        Ok(waves) => Span::raw(format!(
            "ordre : {}",
            waves
                .iter()
                .map(|w| w.iter().map(|i| (i + 1).to_string()).collect::<Vec<_>>().join(" + "))
                .collect::<Vec<_>>()
                .join("  →  ")
        )),
        Err(e) => Span::styled(format!("⚠ {e}"), Style::default().add_modifier(Modifier::BOLD)),
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(order),
            Line::from(Span::styled(
                split.summary.clone(),
                Style::default().add_modifier(Modifier::DIM),
            )),
        ])
        .block(pane_block(&format!("Découpage — {}", detail.epic.title), false))
        .wrap(Wrap { trim: true }),
        chunks[0],
    );

    let width = chunks[1].width.saturating_sub(4) as usize;
    let rows: Vec<Line> = split
        .tickets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let deps = if t.depends_on.is_empty() {
                String::new()
            } else {
                let after: Vec<String> = t.depends_on.iter().map(|d| (d + 1).to_string()).collect();
                format!("  ⛓ après {}", after.join(", "))
            };
            let mut style = Style::default();
            if i == app.split_selected {
                style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
            }
            Line::from(Span::styled(
                truncate(&format!("{}. {}{deps}", i + 1, t.title), width),
                style,
            ))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(rows).block(pane_block("Tickets proposés", true)),
        chunks[1],
    );

    let mut detail_lines = Vec::new();
    if let Some(t) = split.tickets.get(app.split_selected) {
        detail_lines.push(Line::from(t.brief.clone()));
        if !t.acceptance.is_empty() {
            detail_lines.push(Line::from(Span::styled(
                "fini quand :",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for c in &t.acceptance {
                detail_lines.push(Line::from(format!("  ☐ {c}")));
            }
        }
    }
    frame.render_widget(
        Paragraph::new(detail_lines)
            .block(pane_block("Brief — « e » le réécrire", false))
            // Not trimmed: the criteria keep their indent under « fini quand ».
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
}

fn render_epic(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let Some(detail) = app.epic.as_ref() else {
        return;
    };
    let width = area.width.saturating_sub(4) as usize;
    let mut lines = vec![
        Line::from(format!(
            "{} {}",
            symbol(detail.epic.status),
            detail.epic.status.label_fr()
        )),
        Line::from(""),
    ];
    if detail.tickets.is_empty() {
        lines.push(Line::from(Span::styled(
            match detail.epic.status {
                EpicStatus::Draft => {
                    "Pas encore découpée : « p » demande un découpage à l'orchestrateur."
                }
                _ => "Aucun ticket.",
            },
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    for (i, row) in detail.tickets.iter().enumerate() {
        let badge = theme::ticket(row.status);
        let waits: Vec<String> = row
            .depends_on
            .iter()
            .filter(|n| {
                detail.tickets.iter().any(|t| {
                    t.number == **n && t.status != orchestra_core::model::TicketStatus::Done
                })
            })
            .map(|n| format!("#{n}"))
            .collect();
        let waiting = if waits.is_empty() {
            String::new()
        } else {
            format!("  ⛓ attend {}", waits.join(", "))
        };
        let text = format!(
            "#{} {} — {}{waiting}",
            row.number,
            row.title,
            row.status.label_fr()
        );
        let mut style = Style::default();
        if i == app.split_selected {
            style = style.add_modifier(Modifier::REVERSED);
        }
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", badge.symbol), badge.style()),
            Span::styled(truncate(&text, width.saturating_sub(2)), style),
        ]));
    }
    frame.render_widget(
        Paragraph::new(lines).block(pane_block(
            &format!("Épopée — {}", detail.epic.title),
            true,
        )),
        area,
    );
}
