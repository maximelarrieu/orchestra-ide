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

    let status = if app.chat.is_some() {
        Span::styled("💬 conversation", Style::new().magenta().bold())
    } else {
        match app.phase {
            Phase::Idle => Span::styled("● au repos", Style::new().dark_gray()),
            Phase::Running => Span::styled(
                format!("▶ {} agent(s) en cours", app.running_count()),
                Style::new().green().bold(),
            ),
            Phase::Finished => Span::styled(
                format!("✓ terminé ({} agents)", app.done),
                Style::new().green(),
            ),
        }
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
    if app.events.is_empty() {
        let block = Block::bordered().title(" 🛰  ÉCRAN RADAR (FLUX D'ACTIVITÉ DES AGENTS) ");
        let hint = match (app.can_launch(), app.phase) {
            (true, Phase::Idle) => "  Prêt. [1] lancer l'orchestre · [5] converser.",
            (false, _) => "  Aucun agent dans cet espace (ou aucun espace chargé).",
            _ => "  En attente d'activité…",
        };
        let mut lines = vec![
            Line::raw(""),
            Line::from(Span::styled(hint, Style::new().dark_gray())),
        ];
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
        frame.render_widget(Paragraph::new(lines).block(block), area);
        return;
    }

    // On déroule chaque événement en lignes (Markdown + retour à la ligne), puis on affiche
    // une fenêtre : par défaut le bas (auto-scroll), ou plus haut si l'utilisateur a défilé.
    let inner_w = area.width.saturating_sub(2) as usize; // -2 : bordures
    let visible = area.height.saturating_sub(2) as usize;
    let mut rows: Vec<Line> = Vec::new();
    for ev in &app.events {
        event_rows(ev, inner_w, &mut rows);
    }
    let total = rows.len();
    let max_scroll = total.saturating_sub(visible);
    let scroll = app.radar_scroll.min(max_scroll);
    let end = total - scroll;
    let start = end.saturating_sub(visible);

    let title = if scroll > 0 {
        " 🛰  RADAR — ↑ historique · PgDn pour revenir en bas ".to_string()
    } else {
        " 🛰  ÉCRAN RADAR (FLUX D'ACTIVITÉ DES AGENTS) ".to_string()
    };
    let block = Block::bordered().title(title);
    frame.render_widget(Paragraph::new(rows[start..end].to_vec()).block(block), area);
}

/// Style du nom selon l'émetteur (utilisateur / coordinateur / agent).
fn speaker_style(agent: &str) -> Style {
    match agent {
        "Vous" => Style::new().green().bold(),
        "Coordinateur" => Style::new().magenta().bold(),
        _ => Style::new().cyan(),
    }
}

/// Déroule un événement en une ou plusieurs lignes d'affichage (avec retour à la ligne).
fn event_rows(ev: &AgentEvent, width: usize, rows: &mut Vec<Line<'static>>) {
    match ev {
        AgentEvent::Started { agent } => rows.push(Line::from(vec![
            Span::styled("  ▶ ", Style::new().green().bold()),
            Span::styled(agent.clone(), speaker_style(agent).bold()),
            Span::styled(" — démarré", Style::new().dark_gray()),
        ])),
        AgentEvent::Done { agent } => rows.push(Line::from(vec![
            Span::styled("  ✔ ", Style::new().green().bold()),
            Span::styled(agent.clone(), speaker_style(agent).bold()),
            Span::styled(" — terminé", Style::new().green()),
        ])),
        AgentEvent::Log { agent, msg } => {
            let prefix = format!("{agent} : ");
            let indent = 2 + prefix.chars().count();
            let avail = width.saturating_sub(indent);
            // Le message est rendu en Markdown (titres, listes, citations, code) puis chaque
            // bloc est replié à la largeur disponible.
            let mut first = true;
            for (style, text) in markdown::styled_blocks(msg) {
                let mut wrapped = wrap_plain(&text, avail);
                if wrapped.is_empty() {
                    wrapped.push(String::new());
                }
                for chunk in wrapped {
                    if first {
                        rows.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(prefix.clone(), speaker_style(agent)),
                            Span::styled(chunk, style),
                        ]));
                        first = false;
                    } else {
                        rows.push(Line::from(vec![
                            Span::raw(" ".repeat(indent)),
                            Span::styled(chunk, style),
                        ]));
                    }
                }
            }
            if first {
                rows.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(prefix, speaker_style(agent)),
                ]));
            }
        }
    }
}

/// Retour à la ligne « glouton » d'un texte (gère les `\n` et les mots trop longs).
fn wrap_plain(s: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        let mut len = 0usize;
        for word in para.split_whitespace() {
            let wlen = word.chars().count();
            if wlen > width {
                if len > 0 {
                    out.push(std::mem::take(&mut line));
                    len = 0;
                }
                let chars: Vec<char> = word.chars().collect();
                for chunk in chars.chunks(width) {
                    out.push(chunk.iter().collect());
                }
                continue;
            }
            let extra = if len == 0 { wlen } else { wlen + 1 };
            if len + extra > width {
                out.push(std::mem::take(&mut line));
                len = 0;
            }
            if len > 0 {
                line.push(' ');
                len += 1;
            }
            line.push_str(word);
            len += wlen;
        }
        out.push(line); // conserve aussi les lignes vides (paragraphes)
    }
    out
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
    } else if let Some(buf) = &app.chat {
        Line::from(vec![
            Span::styled("› ", Style::new().magenta().bold()),
            Span::raw(buf.clone()),
            Span::styled("▏", Style::new().magenta()),
            Span::styled("   (Entrée = envoyer · Échap = quitter le chat)", Style::new().dark_gray()),
        ])
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
            "[1] Lancer  [5] Converser  [2] Documents  [3] Espace  [4] Persona  [q] Quitter",
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

    #[test]
    fn wrap_plain_wraps_long_text_and_keeps_paragraphs() {
        // Mots normaux : chaque ligne tient dans la largeur.
        let w = wrap_plain("une phrase assez longue à couper en plusieurs lignes", 12);
        assert!(w.iter().all(|l| l.chars().count() <= 12));
        assert!(w.len() > 1, "le texte long est réparti sur plusieurs lignes");
        // Les sauts de paragraphe sont conservés.
        let p = wrap_plain("a\n\nb", 10);
        assert_eq!(p, vec!["a", "", "b"]);
        // Un mot plus long que la largeur est découpé proprement (largeur plancher = 8).
        let long = wrap_plain("supercalifragilistic", 8);
        assert!(long.len() > 1 && long.iter().all(|l| l.chars().count() <= 8));
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
