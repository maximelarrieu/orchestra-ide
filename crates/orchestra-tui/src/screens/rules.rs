//! The Rules screen: the team's conventions and the project's decisions.
//!
//! What every agent is handed on top of its role, and what the daemon checks
//! on a branch before it leaves. A list on the left, the selected rule's text
//! on the right. Proposals from agents land here first, and apply only once
//! accepted.

use orchestra_core::conventions::{Rule, RuleKind};
use orchestra_core::model::RoleScope;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::theme;

use super::{pane_block, truncate};

pub fn render(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let scope = match app.selected_project() {
        Some(p) => format!("Règles — {}", p.name),
        None => "Règles — globales".to_string(),
    };
    if app.rules.is_empty() && app.rule_errors.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "Aucune règle. « : » puis `convention add <titre>` en crée une ; \
                 `adr add <titre>` écrit une décision pour le projet sélectionné.",
            )
            .style(Style::default().add_modifier(Modifier::DIM))
            .block(pane_block(&scope, true))
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let wide = area.width >= 90;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if wide {
            vec![Constraint::Percentage(45), Constraint::Percentage(55)]
        } else {
            vec![Constraint::Percentage(100)]
        })
        .split(area);

    render_list(app, frame, chunks[0], &scope);
    if wide {
        render_body(app.selected_rule(), frame, chunks[1]);
    }
}

fn section(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        label.to_string(),
        Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    ))
}

fn render_list(app: &App, frame: &mut Frame<'_>, area: Rect, title: &str) {
    let width = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    let mut last_kind = None;
    for (i, r) in app.rules.iter().enumerate() {
        if last_kind != Some(r.kind) {
            if last_kind.is_some() {
                lines.push(Line::raw(""));
            }
            lines.push(section(match r.kind {
                RuleKind::Convention => "Conventions",
                RuleKind::Adr => "Décisions d'architecture",
            }));
            last_kind = Some(r.kind);
        }
        let selected = i == app.rule_selected;
        let badge = theme::rule(r.status);
        let mut head = vec![Span::styled(badge.symbol, badge.style()), Span::raw(" ")];
        if !r.checks.is_empty() {
            let gear = theme::measured();
            head.push(Span::styled(gear.symbol, gear.style()));
            head.push(Span::raw(" "));
        }
        let mut style = Style::default();
        if selected {
            style = style.add_modifier(Modifier::BOLD | Modifier::REVERSED);
        }
        let label = match r.kind {
            RuleKind::Adr => format!("{} {}", r.name.split('-').next().unwrap_or(""), r.title),
            RuleKind::Convention => r.title.clone(),
        };
        head.push(Span::styled(truncate(&label, width.saturating_sub(6)), style));
        lines.push(Line::from(head));

        let mut foot = vec![r.status.label_fr().to_string()];
        if r.kind == RuleKind::Convention {
            foot.push(r.audience_fr());
            foot.push(match r.scope {
                RoleScope::Global => "globale".into(),
                RoleScope::Project => "projet".into(),
            });
        }
        if let Some(by) = &r.proposed_by {
            foot.push(format!("par {by}"));
        }
        lines.push(Line::from(Span::styled(
            truncate(&format!("  {}", foot.join(" · ")), width),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    if !app.rule_errors.is_empty() {
        lines.push(Line::raw(""));
        let warn = theme::urgent();
        for e in &app.rule_errors {
            lines.push(Line::from(vec![
                Span::styled(warn.symbol, warn.style()),
                Span::raw(" "),
                Span::raw(truncate(e, width.saturating_sub(2))),
            ]));
        }
    }

    // Keep the selection in view: two lines per rule, plus the headings.
    let inner = area.height.saturating_sub(2) as usize;
    let cursor = selected_line(app);
    let scroll = cursor.saturating_sub(inner.saturating_sub(2)) as u16;
    let title = format!("{title} ({})", app.rules.len());
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll, 0))
            .block(pane_block(&title, true)),
        area,
    );
}

/// The line the selected rule starts on, headings included.
fn selected_line(app: &App) -> usize {
    let mut line = 0;
    let mut last_kind = None;
    for (i, r) in app.rules.iter().enumerate() {
        if last_kind != Some(r.kind) {
            line += if last_kind.is_some() { 2 } else { 1 };
            last_kind = Some(r.kind);
        }
        if i == app.rule_selected {
            return line;
        }
        line += 2;
    }
    line
}

fn render_body(rule: Option<&Rule>, frame: &mut Frame<'_>, area: Rect) {
    let Some(r) = rule else {
        frame.render_widget(Paragraph::new("").block(pane_block("", false)), area);
        return;
    };
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut lines = vec![Line::from(Span::styled(
        r.title.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for check in &r.checks {
        let gear = theme::measured();
        lines.push(Line::from(vec![
            Span::styled(gear.symbol, gear.style()),
            Span::styled(format!(" vérifié : {}", check.label_fr()), dim),
        ]));
    }
    lines.push(Line::from(Span::styled(r.source.display().to_string(), dim)));
    lines.push(Line::raw(""));
    lines.extend(r.body.lines().map(|l| Line::raw(l.to_string())));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(pane_block(&r.name, false)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::conventions::{RuleCheck, RuleMode, RuleStatus};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn rule(name: &str, kind: RuleKind, status: RuleStatus, checked: bool) -> Rule {
        Rule {
            name: name.into(),
            kind,
            title: format!("titre {name}"),
            status,
            applies_to: vec![],
            mode: RuleMode::Any,
            checks: if checked {
                vec![RuleCheck::CommitMessage("^x".into())]
            } else {
                vec![]
            },
            supersedes: None,
            proposed_by: Some("reviewer, ticket #3".into()),
            body: "Le corps de la règle.".into(),
            source: format!("/x/{name}.md").into(),
            scope: RoleScope::Project,
        }
    }

    fn draw(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render(app, f, f.area())).unwrap();
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

    #[test]
    fn an_empty_book_explains_how_to_start() {
        let out = draw(&App::new(), 80, 10);
        assert!(out.contains("Aucune règle"));
    }

    #[test]
    fn both_kinds_are_listed_with_status_and_the_body_of_the_selection() {
        let mut app = App::new();
        app.rules = vec![
            rule("commits", RuleKind::Convention, RuleStatus::Accepted, true),
            rule("paginer", RuleKind::Convention, RuleStatus::Proposed, false),
            rule("0001-sqlite", RuleKind::Adr, RuleStatus::Accepted, false),
        ];
        app.rule_errors = vec!["/x/casse.md : pas d'entête".into()];
        let out = draw(&app, 120, 24);
        assert!(out.contains("Conventions"));
        assert!(out.contains("Décisions d'architecture"));
        assert!(out.contains("0001 titre 0001-sqlite"), "{out}");
        assert!(out.contains("proposée"));
        assert!(out.contains("par reviewer, ticket #3"));
        assert!(out.contains("Le corps de la règle."));
        assert!(out.contains("vérifié"), "le check de la sélection est dit");
        assert!(out.contains("casse.md"));
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        let mut app = App::new();
        app.rules = vec![rule("x", RuleKind::Convention, RuleStatus::Rejected, true)];
        for (w, h) in [(20u16, 6u16), (10, 4), (95, 10)] {
            let _ = draw(&app, w, h);
        }
    }
}
