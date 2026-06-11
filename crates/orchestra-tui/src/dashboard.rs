//! Rendu de la coquille du tableau de bord.
//!
//! Trois zones empilées reproduisant la maquette de la spec : en-tête, écran radar
//! (vide en Phase 1) et menu d'options.

use orchestra_core::model::ContextSpace;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, space: Option<&ContextSpace>) {
    let [header, radar, menu] = Layout::vertical([
        Constraint::Length(3), // en-tête
        Constraint::Min(8),    // écran radar
        Constraint::Length(4), // menu
    ])
    .areas(frame.area());

    render_header(frame, header, space);
    render_radar(frame, radar);
    render_menu(frame, menu);
}

fn render_header(frame: &mut Frame, area: ratatui::layout::Rect, space: Option<&ContextSpace>) {
    let (name, kind) = match space {
        Some(s) => (
            s.config.project_name.clone(),
            s.config.project_type.label(),
        ),
        None => ("Aucun espace chargé".to_string(), "—"),
    };

    let line = Line::from(vec![
        Span::styled("ORCHESTRA IDE v0.1.0", Style::new().bold().cyan()),
        Span::raw("  |  Espace actuel : "),
        Span::styled(format!("[{name}]"), Style::new().yellow().bold()),
        Span::raw(format!("  ({kind})")),
    ]);

    frame.render_widget(Paragraph::new(line).block(Block::bordered()), area);
}

fn render_radar(frame: &mut Frame, area: ratatui::layout::Rect) {
    let block = Block::bordered().title(" 🛰  ÉCRAN RADAR (FLUX D'ACTIVITÉ DES AGENTS) ");
    let body = Paragraph::new(vec![
        Line::raw(""),
        Line::from(Span::styled(
            "  En attente d'activité… (aucun agent branché — Phase 1)",
            Style::new().dark_gray(),
        )),
    ])
    .block(block);
    frame.render_widget(body, area);
}

fn render_menu(frame: &mut Frame, area: ratatui::layout::Rect) {
    let block = Block::bordered().title(" 📋 OPTIONS & MENUS ");
    let line = Line::from(
        "[1] Lancer une intention   [2] Voir les ADRs   [3] Changer d'Espace   [q] Quitter",
    );
    frame.render_widget(Paragraph::new(line).block(block), area);
}
