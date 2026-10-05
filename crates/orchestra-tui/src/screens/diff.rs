//! A ticket's diff, over the screen that asked for it.
//!
//! The files first — what the branch touches, and how much — then the patch.
//! A line says what it is by its first character, as in every diff; the
//! colour only follows it, and never opposes red to green (rule 11).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::app::DiffView;
use crate::theme;

use super::pane_block;

pub fn render(view: &DiffView, frame: &mut Frame<'_>, area: Rect) {
    let d = &view.diff;
    let (added, removed) = d.files.iter().fold((0u32, 0u32), |(a, r), f| {
        (a + f.added.unwrap_or(0), r + f.removed.unwrap_or(0))
    });
    let mut lines: Vec<Line> = Vec::with_capacity(d.files.len() + 4);
    for f in &d.files {
        let counts = match (f.added, f.removed) {
            (Some(a), Some(r)) => format!("{:>6} {:>6}", format!("+{a}"), format!("-{r}")),
            _ => format!("{:>13}", "binaire"),
        };
        lines.push(Line::from(vec![
            Span::styled(counts, Style::default().add_modifier(Modifier::DIM)),
            Span::raw("  "),
            Span::raw(f.path.clone()),
        ]));
    }
    lines.push(Line::from(""));
    for raw in d.patch.lines() {
        lines.push(Line::from(Span::styled(raw.to_string(), theme::diff_line(raw))));
    }
    if d.truncated {
        lines.push(Line::from(Span::styled(
            "… coupé : le reste avec « git diff » dans le worktree",
            Style::default().add_modifier(Modifier::DIM),
        )));
    }

    let height = area.height.saturating_sub(2) as usize;
    view.height.set(height.max(1));
    let start = view.scroll.min(lines.len().saturating_sub(height));
    let shown: Vec<Line> = lines.into_iter().skip(start).take(height).collect();
    let title = format!(
        "Diff — {} ← {} · {} fichier(s), +{added} -{removed}",
        d.base,
        d.branch,
        d.files.len()
    );
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(shown).block(pane_block(&title, true)), area);
}
