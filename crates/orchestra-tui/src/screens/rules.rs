//! The Rules screen: the roles, the team's conventions and the project's
//! decisions.
//!
//! Who the agents are, what every agent is handed on top of its role, and what
//! the daemon checks on a branch before it leaves — one screen, since all of it
//! is « how the team works here ». A list on the left, the selection's text on
//! the right. Proposals from agents land here first, and apply only once
//! accepted.

use orchestra_core::conventions::{Rule, RuleKind};
use orchestra_core::guard::GitPolicy;
use orchestra_core::model::{RoleDefinition, RoleScope};
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
        Some(p) => format!("Rôles & règles — {}", p.name),
        None => "Rôles & règles — globaux".to_string(),
    };
    if app.book_len() == 0 && app.rule_errors.is_empty() && app.role_errors.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "Aucun rôle ni règle. « : » puis `role add <nom>` crée un rôle, \
                 `convention add <titre>` une convention ; `adr add <titre>` écrit une \
                 décision pour le projet sélectionné. `orchestra init` installe les rôles livrés.",
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
        match app.selected_role() {
            Some(role) => render_role(role, frame, chunks[1]),
            None => render_body(app.selected_rule(), frame, chunks[1]),
        }
    }
}

fn section(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        label.to_string(),
        Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    ))
}

fn scope_fr(scope: RoleScope, feminine: bool) -> &'static str {
    match (scope, feminine) {
        (RoleScope::Global, true) => "globale",
        (RoleScope::Global, false) => "global",
        (RoleScope::Project, _) => "projet",
    }
}

fn render_list(app: &App, frame: &mut Frame<'_>, area: Rect, title: &str) {
    let width = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    if !app.roles.is_empty() {
        lines.push(section("Rôles"));
    }
    for (i, r) in app.roles.iter().enumerate() {
        let selected = i == app.rule_selected;
        let git = r.git.unwrap_or_default();
        let mut head = Vec::new();
        if git == GitPolicy::Full {
            let badge = theme::git_open();
            head.push(Span::styled(badge.symbol, badge.style()));
        } else {
            head.push(Span::raw(" "));
        }
        head.push(Span::raw(" "));
        let mut style = Style::default();
        if selected {
            style = style.add_modifier(Modifier::BOLD | Modifier::REVERSED);
        }
        head.push(Span::styled(truncate(&r.name, width.saturating_sub(4)), style));
        lines.push(Line::from(head));

        let mut foot = vec![
            match git {
                GitPolicy::Full => "git complet".to_string(),
                GitPolicy::Confined => "git confiné".to_string(),
            },
            scope_fr(r.scope, false).to_string(),
        ];
        if let Some(model) = &r.model {
            foot.push(model.clone());
        }
        lines.push(Line::from(Span::styled(
            truncate(&format!("  {}", foot.join(" · ")), width),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    if !app.role_errors.is_empty() {
        let warn = theme::urgent();
        for e in &app.role_errors {
            lines.push(Line::from(vec![
                Span::styled(warn.symbol, warn.style()),
                Span::raw(" "),
                Span::raw(truncate(e, width.saturating_sub(2))),
            ]));
        }
    }
    let offset = app.roles.len();
    let mut last_kind = None;
    for (i, r) in app.rules.iter().enumerate() {
        let i = i + offset;
        if last_kind != Some(r.kind) {
            if last_kind.is_some() || !lines.is_empty() {
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
            foot.push(scope_fr(r.scope, true).into());
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
    let title = format!("{title} ({})", app.book_len());
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll, 0))
            .block(pane_block(&title, true)),
        area,
    );
}

/// The line the selection starts on, headings included.
fn selected_line(app: &App) -> usize {
    let mut line = 0;
    if !app.roles.is_empty() {
        line += 1;
        if app.rule_selected < app.roles.len() {
            return line + 2 * app.rule_selected;
        }
        line += 2 * app.roles.len() + app.role_errors.len();
    }
    let mut last_kind = None;
    for (i, r) in app.rules.iter().enumerate() {
        if last_kind != Some(r.kind) {
            line += if line > 0 { 2 } else { 1 };
            last_kind = Some(r.kind);
        }
        if i + app.roles.len() == app.rule_selected {
            return line;
        }
        line += 2;
    }
    line
}

/// What a role is and what it may do, then the instructions it is given.
fn render_role(r: &RoleDefinition, frame: &mut Frame<'_>, area: Rect) {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut lines = Vec::new();
    if !r.description.is_empty() {
        lines.push(Line::from(Span::styled(
            r.description.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
    }
    let git = r.git.unwrap_or_default();
    let mut rights = Vec::new();
    if git == GitPolicy::Full {
        let badge = theme::git_open();
        rights.push(Span::styled(format!("{} ", badge.symbol), badge.style()));
    }
    rights.push(Span::raw(git.label_fr()));
    rights.push(Span::styled("   « p » pour changer", dim));
    lines.push(Line::from(rights));
    let mut facts = Vec::new();
    if let Some(m) = &r.model {
        facts.push(format!("modèle {m}"));
    }
    if let Some(e) = r.effort {
        facts.push(format!("effort {}", e.as_str()));
    }
    if let Some(b) = r.max_budget_usd {
        facts.push(format!("budget {b} $"));
    }
    if !facts.is_empty() {
        lines.push(Line::from(Span::styled(facts.join(" · "), dim)));
    }
    if !r.allowed_tools.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("outils : {}", r.allowed_tools.join(", ")),
            dim,
        )));
    }
    if !r.disallowed_tools.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("interdits : {}", r.disallowed_tools.join(", ")),
            dim,
        )));
    }
    lines.push(Line::from(Span::styled(r.source.display().to_string(), dim)));
    lines.push(Line::raw(""));
    lines.extend(r.system_prompt.lines().map(|l| Line::raw(l.to_string())));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(pane_block(&r.name, false)),
        area,
    );
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
        crate::screens::text_of(w, h, |f| render(app, f, f.area()))
    }

    #[test]
    fn an_empty_book_explains_how_to_start() {
        let out = draw(&App::new(), 80, 10);
        assert!(out.contains("Aucun rôle ni règle"), "{out}");
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

    fn role(name: &str, git: GitPolicy, scope: RoleScope) -> RoleDefinition {
        RoleDefinition {
            name: name.into(),
            description: format!("le rôle {name}"),
            model: Some("sonnet".into()),
            effort: None,
            allowed_tools: vec!["Bash".into()],
            disallowed_tools: vec![],
            max_budget_usd: None,
            subagents: None,
            mcp: vec![],
            max_turns: None,
            tags: vec![],
            git: Some(git),
            system_prompt: format!("Tu es {name}."),
            source: format!("/roles/{name}.md").into(),
            scope,
        }
    }

    fn book() -> App {
        let mut app = App::new();
        app.roles = vec![
            role("backend", GitPolicy::Confined, RoleScope::Global),
            role("integrator", GitPolicy::Full, RoleScope::Project),
        ];
        app.rules = vec![rule("commits", RuleKind::Convention, RuleStatus::Accepted, true)];
        app
    }

    #[test]
    fn roles_come_first_with_their_git_said_in_words() {
        let mut app = book();
        let out = draw(&app, 130, 24);
        assert!(out.contains("Rôles"), "{out}");
        assert!(out.contains("⎇ integrator"), "le symbole accompagne le rôle ouvert : {out}");
        assert!(out.contains("git complet · projet · sonnet"), "{out}");
        assert!(out.contains("git confiné · global"), "{out}");
        assert!(out.contains("Conventions"));
        assert!(out.contains("Tu es backend."), "le corps de la sélection : {out}");
        assert!(out.contains("(3)"), "rôles et règles comptés ensemble : {out}");

        // The cursor walks from the roles into the rules.
        app.rule_selected = 2;
        assert!(app.selected_role().is_none());
        assert_eq!(app.selected_rule().map(|r| r.name.as_str()), Some("commits"));
        let out = draw(&app, 130, 24);
        assert!(out.contains("Le corps de la règle."), "{out}");
    }

    #[test]
    fn opening_git_is_asked_and_closing_it_is_not() {
        use crate::app::Msg;
        use crate::keymap::Action;
        use orchestra_core::protocol::Command;

        let mut app = book();
        app.screen = crate::app::Screen::Rules;
        app.rule_selected = 0;
        let cmds = app.update(Msg::Key(Action::Char('p')));
        assert!(cmds.is_empty(), "rien n'est envoyé avant la réponse");
        let question = app.confirm.as_ref().map(|c| c.question.clone()).unwrap();
        assert!(question.contains("git complet"), "{question}");

        app.confirm = None;
        app.rule_selected = 1;
        let cmds = app.update(Msg::Key(Action::Char('p')));
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetRoleGit { git: GitPolicy::Confined, name, .. }] if name == "integrator"
            ),
            "{cmds:?}"
        );
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
