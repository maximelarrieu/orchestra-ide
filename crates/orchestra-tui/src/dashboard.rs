//! Rendu du tableau de bord.
//!
//! Trois zones empilées (en-tête / écran radar / menu). Depuis la Phase 3, le radar
//! n'est plus une coquille : il affiche en direct le flux d'[`AgentEvent`] agrégé dans
//! l'[`App`].

use orchestra_core::events::AgentEvent;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::{App, Phase};

pub fn render(frame: &mut Frame, app: &App) {
    let [header, radar, menu] = Layout::vertical([
        Constraint::Length(3), // en-tête
        Constraint::Min(8),    // écran radar
        Constraint::Length(4), // menu
    ])
    .areas(frame.area());

    render_header(frame, header, app);
    render_radar(frame, radar, app);
    render_menu(frame, menu);
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
        None => Span::styled("simulé", Style::new().dark_gray()),
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
        Paragraph::new(vec![
            Line::raw(""),
            Line::from(Span::styled(hint, Style::new().dark_gray())),
        ])
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

fn render_menu(frame: &mut Frame, area: Rect) {
    let block = Block::bordered().title(" 📋 OPTIONS & MENUS ");
    let line = Line::from(
        "[1] Lancer l'orchestre   [2] Voir les ADRs   [3] Changer d'Espace   [q] Quitter",
    );
    frame.render_widget(Paragraph::new(line).block(block), area);
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
}
