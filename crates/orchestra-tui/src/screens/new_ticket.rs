//! The new-ticket form: a title and a brief.
//!
//! The brief is what the orchestrator reads, so the form nudges towards a real
//! description rather than a one-liner.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::forms::TicketField;

use super::pane_block;

pub fn render(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);

    let project = app
        .selected_project()
        .map(|p| p.name.as_str())
        .unwrap_or("aucun projet");

    frame.render_widget(
        Paragraph::new(field_text(app, TicketField::Title)).block(pane_block(
            &format!("Titre — projet {project}"),
            app.form_field == TicketField::Title,
        )),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(field_text(app, TicketField::Brief))
            .block(pane_block(
                "Brief — ce que l'orchestrateur lira",
                app.form_field == TicketField::Brief,
            ))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );

    let hint = match &app.form.error {
        Some(e) => Line::from(Span::styled(
            format!("⚠ {e}"),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        None => Line::from(Span::styled(
            "Dis ce qui ne va pas aujourd'hui et ce que tu attends. Plus c'est concret, meilleure est l'équipe proposée.",
            Style::default().add_modifier(Modifier::DIM),
        )),
    };
    frame.render_widget(Paragraph::new(hint).wrap(Wrap { trim: true }), chunks[2]);
}

/// The field's text with a cursor when it has focus.
fn field_text(app: &App, field: TicketField) -> String {
    let text = app.form.field(field);
    if app.form_field == field {
        format!("{text}▏")
    } else {
        text.to_string()
    }
}
