//! The team editor: review, adjust, accept.
//!
//! The orchestrator proposes and the user decides, so every field it chose can
//! be changed here before a single agent starts.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::forms::EditorMode;

use super::pane_block;

pub fn render(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let editor = &app.editor;
    if editor.is_empty() {
        frame.render_widget(
            Paragraph::new("Aucune équipe à relire. Ouvre un ticket et appuie sur « p ».")
                .style(Style::default().add_modifier(Modifier::DIM))
                .block(pane_block("Équipe", true))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    // The detail grows with the criteria it lists, within reason: the team
    // table above is what the screen is for.
    let criteria = editor
        .selected_member()
        .map(|m| m.acceptance.len())
        .unwrap_or(0);
    let detail = if criteria == 0 { 4 } else { (5 + criteria as u16).min(10) };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(5),
            Constraint::Length(detail),
        ])
        .split(area);

    render_summary(app, frame, chunks[0]);
    render_members(app, frame, chunks[1]);
    render_detail(app, frame, chunks[2]);

    if let EditorMode::Objective { buffer } = &editor.mode {
        render_objective_editor(buffer, frame, area);
    }
}

fn render_summary(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let editor = &app.editor;
    let stages = match editor.stages() {
        Ok(stages) => {
            let order = stages
                .iter()
                .map(|stage| stage.join(" + "))
                .collect::<Vec<_>>()
                .join("  →  ");
            Span::styled(format!("ordre : {order}"), Style::default())
        }
        Err(e) => Span::styled(
            format!("⚠ {e}"),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    };
    // The order comes first: a long summary would otherwise push the only line
    // that says whether the team can run at all out of the pane.
    let mut lines = vec![Line::from(stages)];
    if let Some(error) = &editor.error {
        lines.push(Line::from(Span::styled(
            format!("⚠ {error}"),
            Style::default().add_modifier(Modifier::BOLD),
        )));
    }
    if !editor.summary.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            editor.summary.clone(),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane_block("Proposition", false))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_members(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let editor = &app.editor;
    let rows: Vec<Row> = editor
        .members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let style = if i == editor.selected {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                m.role.clone(),
                m.model.clone().unwrap_or_else(|| "défaut".into()),
                m.effort
                    .map(|e| e.as_str().to_string())
                    .unwrap_or_else(|| "défaut".into()),
                if m.depends_on.is_empty() {
                    "-".into()
                } else {
                    m.depends_on.join(", ")
                },
            ])
            .style(style)
        })
        .collect();

    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Min(12),
                Constraint::Length(10),
                Constraint::Length(9),
                Constraint::Min(14),
            ],
        )
        .header(
            Row::new(vec!["rôle", "modèle", "effort", "dépend de"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(pane_block("Équipe", true)),
        area,
    );
}

fn render_detail(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    if let Some(m) = app.editor.selected_member() {
        lines.push(Line::from(m.objective.clone()));
        if !m.acceptance.is_empty() {
            lines.push(Line::from(Span::styled(
                "fini quand :",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for criterion in &m.acceptance {
                lines.push(Line::from(format!("  ☐ {criterion}")));
            }
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane_block("Objectif — « e » pour le réécrire", false))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_objective_editor(buffer: &str, frame: &mut Frame<'_>, area: Rect) {
    let width = area.width.saturating_sub(8).clamp(20, 90);
    let height = area.height.saturating_sub(2).clamp(3, 7);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(format!("{buffer}▏"))
            .block(pane_block("Objectif — Ctrl-S garder, Échap annuler", true))
            .wrap(Wrap { trim: false }),
        popup,
    );
}
