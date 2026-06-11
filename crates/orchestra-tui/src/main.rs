//! Frontend terminal d'Orchestra IDE (Phase 1).
//!
//! Affiche la coquille du tableau de bord en 3 zones (en-tête / écran radar / menu).
//! Aucun LLM, aucun agent : le radar est vide. Le code ne fait que charger un Espace
//! de Contexte via `orchestra-core` et le rendre.

mod dashboard;

use std::path::PathBuf;

use orchestra_core::model::ContextSpace;
use ratatui::crossterm::event::{self, Event, KeyCode};
use ratatui::DefaultTerminal;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // L'espace à ouvrir : 1er argument CLI, sinon le répertoire courant.
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    // On tolère l'absence d'espace : le dashboard s'affiche quand même (état « vide »).
    let space = ContextSpace::load(&root).ok();

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, space);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut DefaultTerminal,
    space: Option<ContextSpace>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        terminal.draw(|frame| dashboard::render(frame, space.as_ref()))?;

        if let Event::Key(key) = event::read()? {
            if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                break;
            }
        }
    }
    Ok(())
}
