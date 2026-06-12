//! Rendu du tableau de bord.
//!
//! Trois zones empilées (en-tête / écran radar / menu). Depuis la Phase 3, le radar
//! n'est plus une coquille : il affiche en direct le flux d'[`AgentEvent`] agrégé dans
//! l'[`App`].

use orchestra_core::events::AgentEvent;
use orchestra_core::model::DocKind;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::{App, Phase, View, Viewer};
use crate::editor::Editor;
use crate::markdown;

pub fn render(frame: &mut Frame, app: &App) {
    let [header, center, menu] = Layout::vertical([
        Constraint::Length(3), // en-tête
        Constraint::Min(8),    // zone centrale (radar / ADRs)
        Constraint::Length(4), // menu
    ])
    .areas(frame.area());

    render_header(frame, header, app);
    if let Some(ed) = &app.editor {
        render_persona_editor(frame, center, ed);
    } else if let Some(v) = &app.viewer {
        render_markdown_viewer(frame, center, v);
    } else {
        match app.view {
            View::Radar => render_radar(frame, center, app),
            View::Docs => render_docs_list(frame, center, app),
        }
    }
    render_menu(frame, menu, app);
}

/// Navigateur des documents de l'espace (persona / ADRs / docs), avec sélection.
fn render_docs_list(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" 📚 DOCUMENTS DE L'ESPACE ");
    let lines: Vec<Line> = if app.docs.is_empty() {
        vec![Line::from(Span::styled(
            "  Aucun document (persona, ADR ou .md du workspace).",
            Style::new().dark_gray(),
        ))]
    } else {
        app.docs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let (tag, tag_style) = match d.kind {
                    DocKind::Persona => ("persona", Style::new().yellow()),
                    DocKind::Adr => ("adr ", Style::new().green()),
                    DocKind::Doc => ("doc ", Style::new().cyan()),
                };
                let selected = i == app.doc_sel;
                let marker = if selected { "▶ " } else { "  " };
                let label_style = if selected {
                    Style::new().bold().reversed()
                } else {
                    Style::new()
                };
                Line::from(vec![
                    Span::raw(marker),
                    Span::styled(format!("[{tag}] "), tag_style),
                    Span::styled(d.label.clone(), label_style),
                ])
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Visualiseur Markdown : rendu stylé + défilement vertical borné.
fn render_markdown_viewer(frame: &mut Frame, area: Rect, v: &Viewer) {
    let block = Block::bordered().title(format!(" 📖 {} ", v.title));
    let rendered = markdown::to_lines(&v.text);
    let visible = area.height.saturating_sub(2) as usize; // -2 : bordures
    let max_scroll = rendered.len().saturating_sub(visible);
    let top = v.scroll.min(max_scroll);
    let lines: Vec<Line> = rendered.into_iter().skip(top).take(visible.max(1)).collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Éditeur de persona : lignes éditables + curseur terminal positionné (avec défilement
/// vertical si le contenu dépasse la zone).
fn render_persona_editor(frame: &mut Frame, area: Rect, ed: &Editor) {
    let dirty = if ed.is_dirty() { " *" } else { "" };
    let block = Block::bordered().title(format!(" ✏ PERSONA (.orchestra/persona.md){dirty} "));

    let inner_h = area.height.saturating_sub(2) as usize; // -2 : bordures
    let (cy, cx) = ed.cursor();
    let top = if inner_h > 0 && cy >= inner_h { cy - inner_h + 1 } else { 0 };

    let lines: Vec<Line> = ed
        .lines()
        .iter()
        .skip(top)
        .take(inner_h.max(1))
        .map(|l| Line::raw(l.iter().collect::<String>()))
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);

    // Curseur terminal (à l'intérieur des bordures), borné à la zone visible.
    let max_x = area.x + area.width.saturating_sub(2);
    let cursor_x = (area.x + 1 + cx as u16).min(max_x);
    let cursor_y = area.y + 1 + (cy - top) as u16;
    frame.set_cursor_position(Position { x: cursor_x, y: cursor_y });
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let (name, kind) = match &app.space {
        Some(s) => (s.config.project_name.clone(), s.config.project_type.label()),
        None => ("Aucun espace chargé".to_string(), "—"),
    };

    let status = match app.phase {
        Phase::Idle => Span::styled("● au repos", Style::new().dark_gray()),
        Phase::Running => Span::styled(
            format!("▶ {} agent(s) en cours", app.running_count()),
            Style::new().green().bold(),
        ),
        Phase::Finished => Span::styled(
            format!("✓ terminé ({} agents)", app.done),
            Style::new().green(),
        ),
    };

    let mode = match &app.llm_model {
        Some(model) => Span::styled(format!("🤖 {model}"), Style::new().magenta()),
        None => Span::styled("simulé · clé API absente", Style::new().yellow()),
    };

    let line = Line::from(vec![
        Span::styled("ORCHESTRA IDE v0.1.0", Style::new().bold().cyan()),
        Span::raw("  |  "),
        Span::styled(format!("[{name}]"), Style::new().yellow().bold()),
        Span::raw(format!(" ({kind})  |  ")),
        mode,
        Span::raw("  |  "),
        status,
    ]);

    frame.render_widget(Paragraph::new(line).block(Block::bordered()), area);
}

fn render_radar(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" 🛰  ÉCRAN RADAR (FLUX D'ACTIVITÉ DES AGENTS) ");

    let body = if app.events.is_empty() {
        let hint = match (app.can_launch(), app.phase) {
            (true, Phase::Idle) => "  Prêt. Appuie sur [1] pour lancer l'orchestre.",
            (false, _) => "  Aucun agent dans cet espace (ou aucun espace chargé).",
            _ => "  En attente d'activité…",
        };
        let mut lines = vec![
            Line::raw(""),
            Line::from(Span::styled(hint, Style::new().dark_gray())),
        ];
        // En mode simulé, rappeler comment activer un vrai LLM.
        if app.llm_model.is_none() {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "  ⚠ Mode simulé — aucune clé API détectée.",
                Style::new().yellow(),
            )));
            lines.push(Line::from(Span::styled(
                "    Définis ANTHROPIC_API_KEY (Claude) ou GEMINI_API_KEY (Gemini) pour activer un vrai LLM.",
                Style::new().dark_gray(),
            )));
        }
        Paragraph::new(lines)
    } else {
        // On n'affiche que les dernières lignes qui tiennent dans la zone (auto-scroll).
        let visible = area.height.saturating_sub(2) as usize; // -2 : bordures
        let start = app.events.len().saturating_sub(visible);
        let lines: Vec<Line> = app.events[start..].iter().map(event_line).collect();
        Paragraph::new(lines)
    };

    frame.render_widget(body.block(block), area);
}

/// Met en forme un événement en ligne de radar, stylé selon sa nature.
fn event_line(ev: &AgentEvent) -> Line<'static> {
    let agent = Style::new().cyan();
    match ev {
        AgentEvent::Started { agent: a } => Line::from(vec![
            Span::styled("  ▶ ", Style::new().green().bold()),
            Span::styled(a.clone(), agent.bold()),
            Span::styled(" — démarré", Style::new().dark_gray()),
        ]),
        AgentEvent::Log { agent: a, msg } => Line::from(vec![
            Span::raw("    "),
            Span::styled(a.clone(), agent),
            Span::raw(" : "),
            Span::raw(msg.clone()),
        ]),
        AgentEvent::Done { agent: a } => Line::from(vec![
            Span::styled("  ✔ ", Style::new().green().bold()),
            Span::styled(a.clone(), agent.bold()),
            Span::styled(" — terminé", Style::new().green()),
        ]),
    }
}

fn render_menu(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" 📋 OPTIONS & MENUS ");

    // Les modes (éditeur / visualiseur / saisie) ont priorité sur le menu.
    let content = if app.editor.is_some() {
        Line::from(Span::styled(
            "✏ Persona — Ctrl+S enregistrer · Échap annuler · ←↑↓→ naviguer",
            Style::new().magenta(),
        ))
    } else if app.viewer.is_some() {
        let edit = if app.viewer_is_persona() { " · [e] éditer" } else { "" };
        Line::from(Span::styled(
            format!("📖 Document — ↑↓ défiler · Échap fermer{edit}"),
            Style::new().cyan(),
        ))
    } else if app.view == View::Docs {
        Line::from(Span::styled(
            "📚 Documents — ↑↓ choisir · Entrée ouvrir · Échap retour",
            Style::new().cyan(),
        ))
    } else if let Some(buf) = &app.input {
        Line::from(vec![
            Span::styled("Chemin de l'espace : ", Style::new().bold()),
            Span::raw(buf.clone()),
            Span::styled("▏", Style::new().cyan()),
            Span::styled("   (Entrée = charger · Échap = annuler)", Style::new().dark_gray()),
        ])
    } else if let Some(notice) = &app.notice {
        Line::from(Span::styled(notice.clone(), Style::new().yellow()))
    } else {
        Line::from(
            "[1] Lancer   [2] Documents   [3] Changer d'Espace   [4] Éditer persona   [q] Quitter",
        )
    };
    frame.render_widget(Paragraph::new(content).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::config::ProjectConfig;
    use orchestra_core::model::project_type::ProjectType;
    use orchestra_core::model::ContextSpace;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn demo_app() -> App {
        let space = ContextSpace {
            root: PathBuf::from("."),
            config: ProjectConfig {
                project_name: "Demo".to_string(),
                project_type: ProjectType::Immobilier,
                workspace_path: None,
                documentalist_enabled: false,
                skills: vec![],
                agents: vec!["Agent_Scraper".to_string()],
                integrations: Default::default(),
            },
            persona: None,
            adrs: vec![],
        };
        let mut app = App::new(Some(space));
        app.begin_run();
        app.on_event(AgentEvent::Started { agent: "Agent_Scraper".into() });
        app.on_event(AgentEvent::Log { agent: "Agent_Scraper".into(), msg: "27 annonces".into() });
        app
    }

    /// Le rendu ne doit jamais paniquer, même quand la zone radar est trop petite pour
    /// l'historique (l'auto-scroll repose sur des `saturating_sub`).
    fn render_at(width: u16, height: u16) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, &demo_app())).unwrap();
    }

    #[test]
    fn renders_without_panic_at_various_sizes() {
        render_at(80, 24);
        render_at(40, 12);
        render_at(20, 6); // radar quasi inexistant
    }

    /// Le navigateur de documents et le mode saisie doivent se rendre sans panique.
    #[test]
    fn renders_docs_view_and_input_mode() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        let mut app = demo_app();
        app.toggle_docs(); // vue Documents
        terminal.draw(|f| render(f, &app)).unwrap();

        app.toggle_docs(); // retour radar
        app.start_space_input(); // invite de saisie dans le menu
        app.input_push('x');
        terminal.draw(|f| render(f, &app)).unwrap();
    }

    /// Le visualiseur Markdown doit se rendre (avec défilement borné) sans panique.
    #[test]
    fn renders_markdown_viewer() {
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        let mut app = demo_app();
        app.viewer = Some(crate::app::Viewer {
            title: "doc.md".into(),
            text: "# Titre\n\n- a\n- b\n\n```\ncode\n```\nfin".into(),
            scroll: 100, // au-delà de la fin → clampé au rendu
            is_persona: false,
        });
        terminal.draw(|f| render(f, &app)).unwrap();
    }

    /// L'éditeur de persona doit se rendre (avec curseur) sans panique.
    #[test]
    fn renders_persona_editor() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut app = demo_app();
        app.open_persona_editor();
        if let Some(ed) = app.editor.as_mut() {
            ed.insert_char('B');
            ed.newline();
        }
        terminal.draw(|f| render(f, &app)).unwrap();
    }
}
