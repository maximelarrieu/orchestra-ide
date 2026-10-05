//! The TODO screen: personal ideas or tasks, unrelated to any project.
//!
//! A capture list, not a kanban: one line for what it is, one for its status
//! and when it is due. `:todo add <titre>` writes; `d` and `p` act on the
//! selected row.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::theme;

use super::{pane_block, truncate};

pub fn render(app: &App, frame: &mut Frame<'_>, area: Rect) {
    if app.todos.is_empty() {
        frame.render_widget(
            Paragraph::new("Aucun todo. « : » puis `todo add <titre>` en crée un.")
                .style(Style::default().add_modifier(Modifier::DIM))
                .block(pane_block("TODO", true))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;
    let per_item = 2;
    let visible = (inner_height / per_item).max(1);
    let start = app.todo_selected.saturating_sub(visible.saturating_sub(1));

    let mut lines: Vec<Line> = Vec::new();
    for (i, t) in app.todos.iter().enumerate().skip(start).take(visible) {
        let selected = i == app.todo_selected;
        let badge = theme::todo(t.status);

        let mut head = vec![Span::styled(badge.symbol, badge.style()), Span::raw(" ")];
        if t.urgent {
            let u = theme::urgent();
            head.push(Span::styled(u.symbol, u.style()));
            head.push(Span::raw(" "));
        }
        let mut title_style = Style::default();
        if selected {
            title_style = title_style.add_modifier(Modifier::BOLD | Modifier::REVERSED);
        }
        head.push(Span::styled(
            truncate(&t.title, width.saturating_sub(4)),
            title_style,
        ));
        lines.push(Line::from(head));

        let mut foot = vec![t.status.label_fr().to_string()];
        if let Some(due) = t.due_at {
            let d = due.date();
            foot.push(format!(
                "échéance {:04}-{:02}-{:02}",
                d.year(),
                u8::from(d.month()),
                d.day()
            ));
        }
        if t.promoted_ticket_id.is_some() {
            foot.push("promu en ticket".to_string());
        }
        lines.push(Line::from(Span::styled(
            truncate(&format!("  {}", foot.join(" · ")), width),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    if start + visible < app.todos.len() {
        lines.push(Line::from(Span::styled(
            format!("  + {} de plus", app.todos.len() - start - visible),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }

    let title = format!("TODO ({})", app.todos.len());
    frame.render_widget(Paragraph::new(lines).block(pane_block(&title, true)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::{Todo, TodoStatus};
    use uuid::Uuid;

    fn todo(title: &str, status: TodoStatus, urgent: bool) -> Todo {
        let now = orchestra_core::now();
        Todo {
            id: Uuid::new_v4(),
            title: title.into(),
            notes: String::new(),
            status,
            urgent,
            due_at: None,
            promoted_ticket_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn draw(app: &App, w: u16, h: u16) -> String {
        crate::screens::text_of(w, h, |f| render(app, f, f.area()))
    }

    #[test]
    fn an_empty_list_explains_itself() {
        let app = App::new();
        let out = draw(&app, 60, 10);
        assert!(out.contains("Aucun todo"));
    }

    #[test]
    fn todos_are_shown_with_their_status() {
        let mut app = App::new();
        app.todos = vec![
            todo("acheter du café", TodoStatus::Open, true),
            todo("relire le rapport", TodoStatus::Done, false),
        ];
        let out = draw(&app, 60, 10);
        assert!(out.contains("acheter du café"));
        assert!(out.contains("relire le rapport"));
        assert!(out.contains("fait"));
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        let mut app = App::new();
        app.todos = vec![todo("x", TodoStatus::Open, false)];
        for (w, h) in [(20u16, 6u16), (10, 4), (40, 10)] {
            let _ = draw(&app, w, h);
        }
    }
}
